//! TURN credentials for one participant, ported from `p2p/services/relayCredentials.ts`: `turn-rest` (an HMAC over
//! the shared secret, which turn-rs and coturn check) or Cloudflare's key API, within the monthly relay quota. Without
//! a credential the participant still gets STUN. Credentials and API tokens are never logged; the provider sees a
//! digest of the account ([`relay_account_tag`]), never the account id.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use base64::Engine as _;
use hmac::{KeyInit, Mac};
use rifts_seam::{AccountId, BoxFuture, Secret, UnixMillis};
use serde_json::Value;
use sha2::Digest;

use crate::config::{
    CloudflareAnalytics, CloudflareTurn, RelayConfig, TurnProvider, TurnRest, is_stun_url, is_turn_url,
};
use crate::wire::{JsonObject, iso_time, js_number_value, json_array, json_string};
use crate::{CredentialIssuer, RelayCredential, RelayError};

pub const CLOUDFLARE_RTC_API: &str = "https://rtc.live.cloudflare.com/v1";
pub const CLOUDFLARE_GRAPHQL_API: &str = "https://api.cloudflare.com/client/v4/graphql";
pub const PROVIDER_TIMEOUT_MS: u64 = 5_000;
const USAGE_CACHE_MS: i64 = 10 * 60_000;
const HOUR_MS: i64 = 3_600_000;

/// `RelayAccountTag`: what the provider sees for an account (Cloudflare's `customIdentifier`, the TURN REST user).
/// Stable, so the usage query finds the account again; account ids are random UUIDs, so it can't be guessed back.
pub fn relay_account_tag(account: &AccountId) -> String {
    let digest = sha2::Sha256::digest(format!("mysticparadox-relay-tag-v1\n{account}"));
    crate::wire::hex(&digest)[..32].to_owned()
}

/// `MintTurnRestCredentials`: the TURN REST scheme (coturn `use-auth-secret`, turn-rs `static-auth-secret`), user
/// `<expiry>:<relay tag>` and password base64(HMAC-SHA1(secret, user)).
pub fn mint_turn_rest(turn_rest: &TurnRest, account: &AccountId, ttl_seconds: u32, now: UnixMillis) -> RelayCredential {
    let expiry = now.0.div_euclid(1000) + i64::from(ttl_seconds);
    let username = format!("{expiry}:{}", relay_account_tag(account));
    RelayCredential {
        credential: Secret::new(turn_rest_password(&turn_rest.secret, &username)),
        username,
        urls: turn_rest.urls.clone(),
        expires_at: UnixMillis(now.0 + i64::from(ttl_seconds) * 1000),
    }
}

/// HMAC accepts a key of any length, so this never fails.
fn turn_rest_password(secret: &Secret, username: &str) -> String {
    match hmac::Hmac::<sha1::Sha1>::new_from_slice(secret.expose().as_bytes()) {
        Ok(mut mac) => {
            mac.update(username.as_bytes());
            base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
        }
        Err(_) => String::new(),
    }
}

/// One POST to a provider: Cloudflare's credential and analytics APIs. The implementation (isles', with its HTTPS
/// client) applies `timeout_ms` and never logs `bearer` or `body`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRequest {
    pub url: String,
    pub bearer: Secret,
    /// JSON text.
    pub body: String,
    pub timeout_ms: u64,
}

/// The status, and the body when it parsed as JSON (`await response.json()` that did not throw).
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderAnswer {
    pub status: u16,
    pub json: Option<Value>,
}

/// The request did not complete: no connection, a timeout, a broken body.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("relay provider request failed")]
pub struct ProviderUnreachable;

pub trait ProviderHttp: Send + Sync {
    fn post(&self, request: ProviderRequest) -> BoxFuture<'_, Result<ProviderAnswer, ProviderUnreachable>>;
}

/// `IceServer`.
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

    /// The reply to the participant, the one place the credential is written out.
    pub fn to_json(&self) -> String {
        JsonObject::new()
            .strings("urls", &self.urls)
            .optional_string("username", self.username.as_deref())
            .optional_string("credential", self.credential.as_ref().map(Secret::expose))
            .finish()
    }
}

/// What a provider's answer holds once read: its STUN URLs and at most one TURN entry with credentials.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProviderIceServers {
    pub stun_urls: Vec<String>,
    pub turn: Option<(Vec<String>, String, Secret)>,
}

/// `^[\x21-\x7e]{1,512}$`
fn credential_text(value: &Value) -> Option<&str> {
    value
        .as_str()
        .filter(|text| (1..=512).contains(&text.len()) && text.bytes().all(|b| (0x21..=0x7e).contains(&b)))
}

/// `/:53(?:\?|$)/`: Cloudflare's alternate port-53 URL, which many networks block and ICE then waits out.
fn port_53(url: &str) -> bool {
    url.ends_with(":53") || url.contains(":53?")
}

fn push_unique(list: &mut Vec<String>, url: &str) {
    if !list.iter().any(|known| known == url) {
        list.push(url.to_owned());
    }
}

/// `NormalizeProviderIceServers`: `iceServers` as an array (generate-ice-servers) or one object (the older API).
pub fn normalize_provider_ice_servers(body: Option<&Value>) -> ProviderIceServers {
    let entries: Vec<&Value> = match body.and_then(|body| body.get("iceServers")) {
        Some(Value::Array(entries)) => entries.iter().collect(),
        Some(Value::Null) | None => Vec::new(),
        Some(single) => vec![single],
    };
    let mut normalized = ProviderIceServers::default();
    for entry in entries {
        let urls: Vec<&str> = match entry.get("urls") {
            Some(Value::String(url)) => vec![url.as_str()],
            Some(Value::Array(urls)) => urls.iter().filter_map(Value::as_str).collect(),
            _ => Vec::new(),
        };
        let urls: Vec<&str> = urls.into_iter().filter(|url| !port_53(url)).collect();
        for url in urls.iter().filter(|url| is_stun_url(url)) {
            push_unique(&mut normalized.stun_urls, url);
        }
        let turn_urls: Vec<String> = urls
            .iter()
            .filter(|url| is_turn_url(url))
            .map(|url| (*url).to_owned())
            .collect();
        if normalized.turn.is_none() && !turn_urls.is_empty() {
            let username = entry.get("username").and_then(credential_text);
            let credential = entry.get("credential").and_then(credential_text);
            if let (Some(username), Some(credential)) = (username, credential) {
                normalized.turn = Some((turn_urls, username.to_owned(), Secret::new(credential.to_owned())));
            }
        }
    }
    normalized
}

/// Why a Cloudflare credential could not be had; the grant answers `RELAY_PROVIDER_ERROR` for each.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProviderFailure {
    #[error("Cloudflare TURN credential request answered {0}")]
    Status(u16),
    #[error("Cloudflare TURN response carried no usable relay entry")]
    NoRelayEntry,
    #[error(transparent)]
    Unreachable(#[from] ProviderUnreachable),
}

/// A provider's credential and the extra STUN URLs it named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuedCredential {
    pub stun_urls: Vec<String>,
    pub credential: RelayCredential,
}

/// `MintCloudflareCredentials`: tagged with the relay tag so usage can be read per account. The tag is not documented
/// for generate-ice-servers; on a 400 the call is repeated without it and that account's quota can't be measured.
pub async fn mint_cloudflare(
    cloudflare: &CloudflareTurn,
    account: &AccountId,
    ttl_seconds: u32,
    now: UnixMillis,
    http: &dyn ProviderHttp,
) -> Result<IssuedCredential, ProviderFailure> {
    // The key id is 8 to 64 letters and digits, so encodeURIComponent leaves it as it is
    let url = format!(
        "{CLOUDFLARE_RTC_API}/turn/keys/{}/credentials/generate-ice-servers",
        cloudflare.key_id
    );
    let request = |body: String| ProviderRequest {
        url: url.clone(),
        bearer: cloudflare.api_token.clone(),
        body,
        timeout_ms: PROVIDER_TIMEOUT_MS,
    };
    let tagged = format!(
        r#"{{"ttl":{ttl_seconds},"customIdentifier":{}}}"#,
        json_string(&relay_account_tag(account))
    );
    let mut answer = http.post(request(tagged)).await?;
    if answer.status == 400 {
        answer = http.post(request(format!(r#"{{"ttl":{ttl_seconds}}}"#))).await?;
    }
    if !(200..300).contains(&answer.status) {
        return Err(ProviderFailure::Status(answer.status));
    }
    let normalized = normalize_provider_ice_servers(answer.json.as_ref());
    let (urls, username, credential) = normalized.turn.ok_or(ProviderFailure::NoRelayEntry)?;
    Ok(IssuedCredential {
        stun_urls: normalized.stun_urls,
        credential: RelayCredential {
            username,
            credential,
            urls,
            expires_at: UnixMillis(now.0 + i64::from(ttl_seconds) * 1000),
        },
    })
}

/// `MonthWindow`: the first day of the month (UTC) and today, as GraphQL date literals, and the month.
pub fn month_window(now: UnixMillis) -> (String, String, String) {
    let iso = iso_time(now);
    let month = iso[..7].to_owned();
    (format!("{month}-01"), iso[..10].to_owned(), month)
}

/// `Number(x) || 0` for one `egressBytes` value.
fn number_or_zero(value: Option<&Value>) -> f64 {
    let number = value.map_or(0.0, js_number_value);
    if number.is_nan() { 0.0 } else { number }
}

/// `ReadMonthlyRelayEgressBytes`: relay bytes Cloudflare sent to this account's clients this month, or `None` when the
/// query fails. A sampled figure that guides the quota, not an invoice.
pub async fn read_monthly_relay_egress_bytes(
    analytics: &CloudflareAnalytics,
    account: &AccountId,
    now: UnixMillis,
    http: &dyn ProviderHttp,
) -> Option<f64> {
    let (from, to, _) = month_window(now);
    // The whitespace is Node's template literal, kept so the query text is the same
    let query = format!(
        "query {{ viewer {{ usage: accounts(filter: {{ accountTag: {} }}) {{\n        callsTurnUsageAdaptiveGroups(filter: {{ date_geq: {}, date_leq: {},\n            customIdentifier: {} }}, limit: 1, orderBy: [customIdentifier_ASC]) {{ sum {{ egressBytes }} }} }} }} }}",
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
    let json = answer.json.as_ref();
    let errors = json
        .and_then(|json| json.get("errors"))
        .and_then(Value::as_array)
        .is_some_and(|errors| !errors.is_empty());
    if answer.status != 200 || errors {
        return None;
    }
    let viewer = json
        .and_then(|json| json.get("data"))
        .and_then(|data| data.get("viewer"));
    let accounts = viewer.and_then(|viewer| match viewer.get("usage") {
        Some(Value::Null) | None => viewer.get("accounts"),
        usage => usage,
    });
    let groups = accounts
        .and_then(Value::as_array)
        .and_then(|accounts| accounts.first())
        .and_then(|first| first.get("callsTurnUsageAdaptiveGroups"))
        .and_then(Value::as_array)?;
    let bytes: f64 = groups
        .iter()
        .map(|group| number_or_zero(group.get("sum").and_then(|sum| sum.get("egressBytes"))))
        .sum();
    (bytes.is_finite() && bytes >= 0.0).then_some(bytes)
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// `RelayUsageCache`: per process, so a burst of requests asks Cloudflare once.
#[derive(Debug, Default)]
pub struct RelayUsageCache {
    entries: Mutex<HashMap<AccountId, (String, f64, i64)>>,
}

impl RelayUsageCache {
    pub fn get(&self, account: &AccountId, month: &str, now: UnixMillis) -> Option<f64> {
        lock(&self.entries)
            .get(account)
            .filter(|(cached_month, _, fetched_at)| cached_month == month && now.0 - fetched_at < USAGE_CACHE_MS)
            .map(|(_, bytes, _)| *bytes)
    }

    pub fn set(&self, account: &AccountId, month: &str, bytes: f64, now: UnixMillis) {
        let mut entries = lock(&self.entries);
        if entries.len() > 10_000 {
            entries.clear();
        }
        entries.insert(account.clone(), (month.to_owned(), bytes, now.0));
    }
}

/// `HourlyCounter`: a sliding hour per key, in process; the request rate limiter still covers the route.
#[derive(Debug, Default)]
pub struct HourlyCounter {
    hits: Mutex<HashMap<String, Vec<i64>>>,
}

impl HourlyCounter {
    pub fn allow(&self, key: &str, limit: u32, now: UnixMillis) -> bool {
        let mut hits = lock(&self.hits);
        let mut recent: Vec<i64> = hits
            .get(key)
            .map(|times| times.iter().copied().filter(|at| now.0 - at < HOUR_MS).collect())
            .unwrap_or_default();
        if recent.len() >= limit as usize {
            hits.insert(key.to_owned(), recent);
            return false;
        }
        recent.push(now.0);
        hits.insert(key.to_owned(), recent);
        if hits.len() > 10_000 {
            hits.retain(|_, times| times.iter().any(|at| now.0 - at < HOUR_MS));
        }
        true
    }
}

/// `RelayDeniedReason`.
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

/// `RelayGrant`: the ICE servers for one participant and whether a relay credential is among them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayGrant {
    pub ice_servers: Vec<IceServer>,
    pub relay: RelayStatus,
}

impl RelayGrant {
    fn stun_only(config: &RelayConfig, reason: RelayDenied) -> Self {
        Self {
            ice_servers: vec![IceServer::stun(config.stun_urls.clone())],
            relay: RelayStatus::Denied(reason),
        }
    }

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

    /// `ISSUED` or the denial, for the metric and the log line.
    pub fn result_label(&self) -> &'static str {
        match self.relay {
            RelayStatus::Allowed { .. } => "ISSUED",
            RelayStatus::Denied(reason) => reason.as_str(),
        }
    }
}

/// The `turn-rest` provider: turn-rs (DD24), or coturn as its fallback.
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
        Box::pin(async move {
            Ok(IssuedCredential {
                stun_urls: Vec::new(),
                credential,
            })
        })
    }
}

/// Cloudflare TURN, a setting (DD24).
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

/// `IssueRelayGrant` with its dependencies: the provider HTTP client and the usage cache.
#[derive(Clone)]
pub struct RelayGrants {
    pub config: Arc<RelayConfig>,
    pub http: Arc<dyn ProviderHttp>,
    pub usage: Arc<RelayUsageCache>,
}

impl RelayGrants {
    /// The issuer for the configured provider, when it has its keys.
    pub fn issuer(&self) -> Option<Box<dyn CredentialIssuer>> {
        match self.config.provider {
            TurnProvider::Cloudflare => self.config.cloudflare.clone().map(|key| {
                Box::new(CloudflareIssuer {
                    key,
                    http: Arc::clone(&self.http),
                }) as Box<dyn CredentialIssuer>
            }),
            TurnProvider::TurnRest => self
                .config
                .turn_rest
                .clone()
                .map(|rest| Box::new(TurnRestIssuer(rest)) as Box<dyn CredentialIssuer>),
            TurnProvider::None => None,
        }
    }

    /// Never fails for provider trouble: the answer then carries STUN only.
    pub async fn grant(&self, account: &AccountId, ttl_seconds: u32, now: UnixMillis) -> RelayGrant {
        let config = self.config.as_ref();
        let Some(issuer) = self.issuer() else {
            return RelayGrant::stun_only(config, RelayDenied::NotConfigured);
        };
        let measured = config
            .analytics
            .as_ref()
            .filter(|_| config.provider == TurnProvider::Cloudflare && config.monthly_quota_bytes > 0);
        if let Some(analytics) = measured {
            let (_, _, month) = month_window(now);
            let mut used = self.usage.get(account, &month, now);
            if used.is_none() {
                used = read_monthly_relay_egress_bytes(analytics, account, now, self.http.as_ref()).await;
                // An unreadable figure does not block play; the per-hour request limit still applies
                if let Some(bytes) = used {
                    self.usage.set(account, &month, bytes, now);
                }
            }
            // The quota is at most 1e14, well inside f64's exact integers
            if used.is_some_and(|bytes| bytes >= config.monthly_quota_bytes as f64) {
                return RelayGrant::stun_only(config, RelayDenied::QuotaExceeded);
            }
        }
        match issuer.issue(account.clone(), ttl_seconds, now).await {
            Ok(issued) => {
                let mut stun = config.stun_urls.clone();
                for url in &issued.stun_urls {
                    push_unique(&mut stun, url);
                }
                RelayGrant {
                    ice_servers: vec![IceServer::stun(stun), IceServer::turn(&issued.credential)],
                    relay: RelayStatus::Allowed {
                        ttl_seconds,
                        expires_at: UnixMillis(now.0 + i64::from(ttl_seconds) * 1000),
                    },
                }
            }
            Err(_) => RelayGrant::stun_only(config, RelayDenied::ProviderError),
        }
    }
}

/// `RelayTtlSeconds`: the hunt's remaining lifetime plus 10 minutes, within 300 seconds and the cap.
pub fn relay_ttl_seconds(hunt_expires_at: UnixMillis, now: UnixMillis, max_ttl_seconds: u32) -> u32 {
    // Math.ceil of a quotient that may be negative
    let remaining = -(-(hunt_expires_at.0 - now.0)).div_euclid(1000);
    let wanted = remaining.saturating_add(600);
    let bounded = wanted.max(300).min(i64::from(max_ttl_seconds));
    u32::try_from(bounded).unwrap_or(max_ttl_seconds)
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::{HashMap, VecDeque};

    use rifts_seam::poll_once;

    use super::*;

    pub(crate) const NOW: UnixMillis = UnixMillis(1_791_878_400_000); // 2026-10-13T08:00:00.000Z
    const ACCOUNT: &str = "4f6a2d1e-0000-4000-8000-000000000001";

    fn account() -> AccountId {
        AccountId(ACCOUNT.to_owned())
    }

    fn hex96() -> String {
        "a".repeat(96)
    }

    /// The documented generate-ice-servers answer (developers.cloudflare.com/realtime/turn/generate-credentials).
    fn cloudflare_answer() -> Value {
        serde_json::json!({
            "iceServers": [
                { "urls": ["stun:stun.cloudflare.com:3478"] },
                { "urls": ["turn:turn.cloudflare.com:3478?transport=udp", "turn:turn.cloudflare.com:53?transport=udp",
                    "turn:turn.cloudflare.com:443?transport=udp", "turns:turn.cloudflare.com:5349?transport=tcp"],
                  "username": hex96(), "credential": hex96() }
            ]
        })
    }

    /// The test double for Cloudflare: queued answers (500 when none is left), every request kept.
    #[derive(Default)]
    pub(crate) struct FakeHttp {
        answers: Mutex<VecDeque<Result<ProviderAnswer, ProviderUnreachable>>>,
        pub(crate) calls: Mutex<Vec<ProviderRequest>>,
    }

    impl FakeHttp {
        pub(crate) fn answering(answers: Vec<(u16, Option<Value>)>) -> Arc<Self> {
            let answers = answers
                .into_iter()
                .map(|(status, json)| Ok(ProviderAnswer { status, json }))
                .collect();
            Arc::new(Self {
                answers: Mutex::new(answers),
                calls: Mutex::new(Vec::new()),
            })
        }

        pub(crate) fn bodies(&self) -> Vec<Value> {
            lock(&self.calls)
                .iter()
                .map(|call| serde_json::from_str(&call.body).unwrap())
                .collect()
        }
    }

    impl ProviderHttp for FakeHttp {
        fn post(&self, request: ProviderRequest) -> BoxFuture<'_, Result<ProviderAnswer, ProviderUnreachable>> {
            lock(&self.calls).push(request);
            let answer = lock(&self.answers).pop_front().unwrap_or(Ok(ProviderAnswer {
                status: 500,
                json: None,
            }));
            Box::pin(async move { answer })
        }
    }

    fn settings(pairs: &[(&str, &str)]) -> RelayConfig {
        let map: HashMap<String, String> = pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect();
        RelayConfig::from_settings(&map)
    }

    fn cloudflare(extra: &[(&str, &str)]) -> RelayConfig {
        let mut pairs = vec![
            ("P2P_CLOUDFLARE_TURN_KEY_ID", "0123456789abcdef0123456789abcdef"),
            ("P2P_CLOUDFLARE_TURN_API_TOKEN", "token-value-123456"),
        ];
        pairs.extend_from_slice(extra);
        settings(&pairs)
    }

    fn analytics() -> [(&'static str, &'static str); 3] {
        [
            (
                "P2P_CLOUDFLARE_ANALYTICS_ACCOUNT_ID",
                "abcdef0123456789abcdef0123456789",
            ),
            ("P2P_CLOUDFLARE_ANALYTICS_API_TOKEN", "analytics-token-1"),
            ("P2P_RELAY_MONTHLY_QUOTA_GB", "10"),
        ]
    }

    fn grants(config: RelayConfig, http: Arc<FakeHttp>, usage: Arc<RelayUsageCache>) -> RelayGrants {
        RelayGrants {
            config: Arc::new(config),
            http,
            usage,
        }
    }

    fn grant(grants: &RelayGrants) -> RelayGrant {
        poll_once(grants.grant(&account(), 600, NOW)).unwrap()
    }

    #[test]
    fn provider_answers_both_shapes_port_53_dropped_bad_credentials_refused() {
        let array = normalize_provider_ice_servers(Some(&cloudflare_answer()));
        assert_eq!(array.stun_urls, ["stun:stun.cloudflare.com:3478"]);
        let (urls, username, _) = array.turn.unwrap();
        assert_eq!(
            urls,
            [
                "turn:turn.cloudflare.com:3478?transport=udp",
                "turn:turn.cloudflare.com:443?transport=udp",
                "turns:turn.cloudflare.com:5349?transport=tcp"
            ]
        );
        assert_eq!(username, hex96());
        let single = normalize_provider_ice_servers(Some(&serde_json::json!({ "iceServers": {
            "urls": ["stun:stun.cloudflare.com:3478", "turn:turn.cloudflare.com:3478?transport=udp"],
            "username": hex96(), "credential": hex96() } })));
        assert_eq!(single.turn.unwrap().0, ["turn:turn.cloudflare.com:3478?transport=udp"]);
        let spaced = serde_json::json!({ "iceServers": [{ "urls": ["turn:turn.cloudflare.com:3478"],
            "username": "a b", "credential": hex96() }] });
        assert_eq!(normalize_provider_ice_servers(Some(&spaced)).turn, None);
        assert_eq!(normalize_provider_ice_servers(None).turn, None);
        assert_eq!(normalize_provider_ice_servers(Some(&Value::Null)).turn, None);
    }

    #[test]
    fn turn_rest_credentials_are_expiry_and_relay_tag_signed_with_the_shared_secret() {
        let secret = "x".repeat(16);
        let rest = TurnRest {
            urls: vec!["turn:turn.example.org:3478".to_owned()],
            secret: Secret::new(secret.clone()),
        };
        let turn = mint_turn_rest(&rest, &account(), 600, NOW);
        assert_eq!(
            turn.username,
            format!("{}:{}", NOW.0 / 1000 + 600, relay_account_tag(&account()))
        );
        let mut mac = hmac::Hmac::<sha1::Sha1>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(turn.username.as_bytes());
        let expected = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
        assert_eq!(turn.credential.expose(), expected);
        // RFC 2202 test case 2, so the HMAC is the one coturn and turn-rs compute
        assert_eq!(
            turn_rest_password(&Secret::new("Jefe".to_owned()), "what do ya want for nothing?"),
            base64::engine::general_purpose::STANDARD.encode([
                0xef, 0xfc, 0xdf, 0x6a, 0xe5, 0xeb, 0x2f, 0xa2, 0xd2, 0x74, 0x16, 0xd5, 0xf1, 0x84, 0xdf, 0x9c, 0x25,
                0x9a, 0x7c, 0x79
            ])
        );
        assert_eq!(turn.expires_at, UnixMillis(NOW.0 + 600_000));
    }

    #[test]
    fn relay_tag_is_stable_32_hex_and_never_the_account_id() {
        let tag = relay_account_tag(&account());
        assert_eq!(tag, relay_account_tag(&account()));
        assert_eq!(tag.len(), 32);
        assert!(tag.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
        assert_ne!(
            relay_account_tag(&AccountId("4f6a2d1e-0000-4000-8000-000000000002".to_owned())),
            tag
        );
        assert!(!tag.contains(ACCOUNT));
    }

    #[test]
    fn no_request_to_the_provider_carries_the_raw_account_id() {
        let http = FakeHttp::answering(vec![
            (200, Some(serde_json::json!({ "data": { "viewer": { "usage": [] } } }))),
            (201, Some(cloudflare_answer())),
        ]);
        let grants = grants(cloudflare(&analytics()), http.clone(), Arc::default());
        assert!(matches!(grant(&grants).relay, RelayStatus::Allowed { .. }));
        let calls = lock(&http.calls);
        assert_eq!(calls.len(), 2);
        for call in calls.iter() {
            assert!(
                !call.body.contains(ACCOUNT) && !call.url.contains(ACCOUNT),
                "{}",
                call.url
            );
        }
        drop(calls);
        let query = http.bodies()[0]["query"].as_str().unwrap().to_owned();
        assert!(query.contains(&format!(r#"customIdentifier: "{}""#, relay_account_tag(&account()))));
    }

    #[test]
    fn stun_only_until_a_provider_is_configured() {
        let grant = grant(&grants(settings(&[]), FakeHttp::answering(vec![]), Arc::default()));
        assert_eq!(
            grant.ice_servers_json(),
            r#"[{"urls":["stun:stun.cloudflare.com:3478"]}]"#
        );
        assert_eq!(
            grant.relay_json(),
            r#"{"allowed":false,"reason":"RELAY_NOT_CONFIGURED"}"#
        );
    }

    #[test]
    fn cloudflare_credentials_are_tagged_and_retried_untagged_on_400() {
        let http = FakeHttp::answering(vec![(201, Some(cloudflare_answer()))]);
        let first = grant(&grants(cloudflare(&[]), http.clone(), Arc::default()));
        let call = lock(&http.calls)[0].clone();
        assert_eq!(
            call.url,
            "https://rtc.live.cloudflare.com/v1/turn/keys/0123456789abcdef0123456789abcdef/credentials/generate-ice-servers"
        );
        assert_eq!(call.bearer.expose(), "token-value-123456");
        assert_eq!(
            call.body,
            format!(
                r#"{{"ttl":600,"customIdentifier":"{}"}}"#,
                relay_account_tag(&account())
            )
        );
        assert_eq!(call.timeout_ms, 5_000);
        assert_eq!(first.ice_servers[1].username.as_deref(), Some(hex96().as_str()));
        assert_eq!(
            first.relay_json(),
            r#"{"allowed":true,"ttlSeconds":600,"expiresAt":"2026-10-13T08:10:00.000Z"}"#
        );

        let retried = FakeHttp::answering(vec![(400, None), (201, Some(cloudflare_answer()))]);
        let second = grant(&grants(cloudflare(&[]), retried.clone(), Arc::default()));
        assert_eq!(
            retried.bodies(),
            [
                serde_json::json!({ "ttl": 600, "customIdentifier": relay_account_tag(&account()) }),
                serde_json::json!({ "ttl": 600 })
            ]
        );
        assert!(matches!(second.relay, RelayStatus::Allowed { .. }));

        let failed = grant(&grants(
            cloudflare(&[]),
            FakeHttp::answering(vec![(503, None)]),
            Arc::default(),
        ));
        assert_eq!(failed.relay, RelayStatus::Denied(RelayDenied::ProviderError));
        assert_eq!(
            failed.ice_servers_json(),
            r#"[{"urls":["stun:stun.cloudflare.com:3478"]}]"#
        );
    }

    #[test]
    fn the_monthly_quota_comes_from_analytics_cached_and_an_unreadable_figure_does_not_block() {
        let usage = |bytes: u64| {
            (
                200,
                Some(serde_json::json!({ "data": { "viewer": { "usage": [
                { "callsTurnUsageAdaptiveGroups": [{ "sum": { "egressBytes": bytes } }] }] } } })),
            )
        };
        let cache = Arc::new(RelayUsageCache::default());
        let http = FakeHttp::answering(vec![usage(10_000_000_001)]);
        let over = grant(&grants(cloudflare(&analytics()), http.clone(), cache.clone()));
        assert_eq!(over.relay, RelayStatus::Denied(RelayDenied::QuotaExceeded));
        let query = http.bodies()[0]["query"].as_str().unwrap().to_owned();
        assert!(query.contains(&format!(r#"customIdentifier: "{}""#, relay_account_tag(&account()))));
        assert!(query.contains(r#"date_geq: "2026-10-01", date_leq: "2026-10-13""#));
        assert_eq!(lock(&http.calls)[0].bearer.expose(), "analytics-token-1");
        // Cached: the second grant does not ask again (the queued answer would be used otherwise)
        let again = grant(&grants(
            cloudflare(&analytics()),
            FakeHttp::answering(vec![(201, Some(cloudflare_answer()))]),
            cache,
        ));
        assert_eq!(again.relay, RelayStatus::Denied(RelayDenied::QuotaExceeded));

        let unreadable = grant(&grants(
            cloudflare(&analytics()),
            FakeHttp::answering(vec![(500, None), (201, Some(cloudflare_answer()))]),
            Arc::default(),
        ));
        assert!(matches!(unreadable.relay, RelayStatus::Allowed { .. }));
    }

    #[test]
    fn the_analytics_query_text_is_nodes() {
        let http = FakeHttp::answering(vec![]);
        let analytics = CloudflareAnalytics {
            account_id: "abcdef0123456789abcdef0123456789".to_owned(),
            api_token: Secret::new("analytics-token-1".to_owned()),
        };
        assert_eq!(
            poll_once(read_monthly_relay_egress_bytes(
                &analytics,
                &account(),
                NOW,
                http.as_ref()
            )),
            Some(None)
        );
        let expected = format!(
            "query {{ viewer {{ usage: accounts(filter: {{ accountTag: \"abcdef0123456789abcdef0123456789\" }}) {{\n        callsTurnUsageAdaptiveGroups(filter: {{ date_geq: \"2026-10-01\", date_leq: \"2026-10-13\",\n            customIdentifier: \"{}\" }}, limit: 1, orderBy: [customIdentifier_ASC]) {{ sum {{ egressBytes }} }} }} }} }}",
            relay_account_tag(&account())
        );
        assert_eq!(http.bodies()[0]["query"], expected);
        assert_eq!(lock(&http.calls)[0].url, CLOUDFLARE_GRAPHQL_API);
    }

    #[test]
    fn usage_figures_read_as_node_sums_them() {
        let read = |json: Value| {
            let http = FakeHttp::answering(vec![(200, Some(json))]);
            let analytics = CloudflareAnalytics {
                account_id: "abcdef0123456789".to_owned(),
                api_token: Secret::new("analytics-token-1".to_owned()),
            };
            poll_once(read_monthly_relay_egress_bytes(
                &analytics,
                &account(),
                NOW,
                http.as_ref(),
            ))
            .unwrap()
        };
        let groups = |groups: Value| serde_json::json!({ "data": { "viewer": { "accounts": [{ "callsTurnUsageAdaptiveGroups": groups }] } } });
        assert_eq!(
            read(groups(
                serde_json::json!([{ "sum": { "egressBytes": "5" } }, { "sum": { "egressBytes": 7 } }])
            )),
            Some(12.0)
        );
        assert_eq!(
            read(groups(serde_json::json!([{ "sum": { "egressBytes": "x" } }]))),
            Some(0.0)
        );
        assert_eq!(
            read(groups(serde_json::json!([{ "sum": { "egressBytes": -3 } }]))),
            None
        );
        assert_eq!(read(groups(serde_json::json!({}))), None);
        assert_eq!(read(serde_json::json!({ "errors": [{}], "data": {} })), None);
    }

    #[test]
    fn hourly_counter_month_window_and_credential_lifetime() {
        let counter = HourlyCounter::default();
        assert!(counter.allow("a", 2, UnixMillis(0)));
        assert!(counter.allow("a", 2, UnixMillis(1)));
        assert!(!counter.allow("a", 2, UnixMillis(2)));
        assert!(counter.allow("a", 2, UnixMillis(3_600_001)));
        assert_eq!(
            month_window(NOW),
            ("2026-10-01".to_owned(), "2026-10-13".to_owned(), "2026-10".to_owned())
        );
        // A hunt expiring in an hour: one hour plus the 10-minute grace
        assert_eq!(relay_ttl_seconds(UnixMillis(NOW.0 + 3_600_000), NOW, 10_800), 4_200);
        assert_eq!(
            relay_ttl_seconds(UnixMillis(NOW.0 + 12 * 3_600_000), NOW, 10_800),
            10_800
        );
        assert_eq!(relay_ttl_seconds(UnixMillis(NOW.0 - 3_600_000), NOW, 10_800), 300);
        assert_eq!(relay_ttl_seconds(UnixMillis(NOW.0 + 1_500), NOW, 10_800), 602);
    }

    #[test]
    fn both_providers_issue_through_the_issuer_trait() {
        let rest = settings(&[
            ("P2P_TURN_PROVIDER", "turn-rest"),
            (
                "P2P_TURN_REST_URLS",
                "turn:relay.example.org:3478?transport=udp,turns:relay.example.org:5349",
            ),
            ("P2P_TURN_REST_SECRET", "a-shared-secret-of-32-characters"),
        ]);
        let grants_rest = grants(rest, FakeHttp::answering(vec![]), Arc::default());
        let issuer = grants_rest.issuer().unwrap();
        assert_eq!(issuer.provider(), TurnProvider::TurnRest);
        let issued = poll_once(issuer.issue(account(), 600, NOW)).unwrap().unwrap();
        assert!(
            issued
                .credential
                .username
                .starts_with(&format!("{}:", NOW.0 / 1000 + 600))
        );
        let grant = grant(&grants_rest);
        assert_eq!(grant.ice_servers.len(), 2);
        assert_eq!(
            grant.ice_servers[1].urls,
            [
                "turn:relay.example.org:3478?transport=udp",
                "turns:relay.example.org:5349"
            ]
        );
        assert!(!format!("{grant:?}").contains(grant.ice_servers[1].credential.as_ref().unwrap().expose()));

        let http = FakeHttp::answering(vec![(201, Some(cloudflare_answer()))]);
        let cloudflare_grants = grants(cloudflare(&[]), http, Arc::default());
        let issuer = cloudflare_grants.issuer().unwrap();
        assert_eq!(issuer.provider(), TurnProvider::Cloudflare);
        let issued = poll_once(issuer.issue(account(), 600, NOW)).unwrap().unwrap();
        assert_eq!(issued.credential.username, hex96());
        assert!(
            grants(
                settings(&[("P2P_TURN_PROVIDER", "none")]),
                FakeHttp::answering(vec![]),
                Arc::default()
            )
            .issuer()
            .is_none()
        );
    }
}
