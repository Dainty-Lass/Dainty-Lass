// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! TURN credentials: minted here for a turn-rest server, or asked of Cloudflare, with the monthly relay quota read
//! from Cloudflare's analytics. Providers see a hashed tag, never an account id.

use std::{
    collections::HashMap,
    fmt,
    future::ready,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use hmac::{Hmac, KeyInit as _, Mac as _};
use rustic_rifts_seam::{AccountId, BoxFuture, Secret, UnixMillis};
use serde_json::Value;
use sha2::Digest as _;

use crate::{
    Answer, CredentialIssuer, RelayCredential,
    config::{CloudflareAnalytics, CloudflareTurn, RelayConfig, TurnProvider, TurnRest, is_stun_url, is_turn_url},
    wire::{self, JsonObject},
};

/// How long a provider request may take, in milliseconds.
pub const PROVIDER_TIMEOUT_MS: u64 = 5000;
/// Cloudflare's Realtime TURN API.
pub const CLOUDFLARE_RTC_API: &str = "https://rtc.live.cloudflare.com/v1";
/// Cloudflare's GraphQL analytics API, read for relay usage.
pub const CLOUDFLARE_GRAPHQL_API: &str = "https://api.cloudflare.com/client/v4/graphql";

// Umbrella spec D3: the Rustic label. Changing it changes every tag, and so the provider's per-tag usage history.
const RELAY_TAG_LABEL: &str = "rustic-relay-tag-v1";
/// How long a usage figure is trusted, the counters' window, and how many keys a map holds before it is pruned.
const USAGE_FRESH_MS: i64 = 10 * 60 * 1000;
const WINDOW_MS: i64 = 60 * 60 * 1000;
const MAX_TRACKED: usize = 10_000;

fn guard<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // The maps hold plain counters; a panic elsewhere leaves them usable.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// `now` plus `ttl_seconds`, as the expiry every credential and grant reports.
fn after(now: UnixMillis, ttl_seconds: u32) -> UnixMillis {
    UnixMillis(now.0 + 1000 * i64::from(ttl_seconds))
}

/// What a provider is told instead of the account id.
pub fn relay_account_tag(account: &AccountId) -> String {
    let digest = sha2::Sha256::new()
        .chain_update(RELAY_TAG_LABEL)
        .chain_update("\n")
        .chain_update(&account.0)
        .finalize();
    // 16 bytes of the digest, as 32 hex digits.
    hex::encode(&digest[..16])
}

/// A TURN REST credential (coturn's `use-auth-secret`, turn-rs's `static-auth-secret`).
pub fn mint_turn_rest(turn_rest: &TurnRest, account: &AccountId, ttl_seconds: u32, now: UnixMillis) -> RelayCredential {
    let valid_until = now.0.div_euclid(1000) + i64::from(ttl_seconds);
    let username = [valid_until.to_string(), relay_account_tag(account)].join(":");
    // HMAC takes a key of any length, so the empty fallback is never reached.
    let password = <Hmac<sha1::Sha1>>::new_from_slice(turn_rest.secret.expose().as_bytes()).map_or_else(
        |_| String::new(),
        |mac| STANDARD.encode(mac.chain_update(&username).finalize().into_bytes()),
    );
    RelayCredential {
        credential: Secret::new(password),
        expires_at: after(now, ttl_seconds),
        urls: turn_rest.urls.clone(),
        username,
    }
}

/// A credential as a provider issued it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IssuedCredential {
    /// STUN URLs the provider adds to the configured ones.
    pub stun_urls: Vec<String>,
    /// The TURN credential.
    pub credential: RelayCredential,
}

/// Issues TURN REST (shared-secret) credentials locally, with no provider call.
#[derive(Clone, Debug)]
pub struct TurnRestIssuer(pub TurnRest);

impl CredentialIssuer for TurnRestIssuer {
    fn provider(&self) -> TurnProvider {
        TurnProvider::TurnRest
    }

    fn issue(&self, to: AccountId, ttl_seconds: u32, at: UnixMillis) -> Answer<'_, IssuedCredential> {
        let issued = IssuedCredential {
            credential: mint_turn_rest(&self.0, &to, ttl_seconds, at),
            stun_urls: Vec::new(),
        };
        Box::pin(ready(Ok(issued)))
    }
}

/// One HTTPS POST to the relay provider.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderRequest {
    /// The full URL.
    pub url: String,
    /// The API token, sent as `Authorization: Bearer`.
    pub bearer: Secret,
    /// JSON text.
    pub body: String,
    /// How long the request may take, in milliseconds.
    pub timeout_ms: u64,
}

/// What the provider answered.
#[derive(Clone, Debug, PartialEq)]
pub struct ProviderAnswer {
    /// The HTTP status.
    pub status: u16,
    /// `None` when the body was not JSON.
    pub json: Option<Value>,
}

/// The provider could not be reached, or did not answer in time.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderUnreachable;

impl fmt::Display for ProviderUnreachable {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str("relay provider request failed")
    }
}

impl std::error::Error for ProviderUnreachable {}

/// HTTPS to the provider, written by the module: a POST with `Authorization: Bearer`, the JSON body and the
/// timeout. It logs neither the token nor the body.
pub trait ProviderHttp: Send + Sync {
    /// Sends the request; `Err` only when no answer came back.
    fn post(&self, request: ProviderRequest) -> BoxFuture<'_, Result<ProviderAnswer, ProviderUnreachable>>;
}

/// Why a provider call gave no credential.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProviderFailure {
    /// The provider answered with a status that is not success.
    Status(u16),
    /// The answer had no usable TURN entry.
    NoRelayEntry,
    /// The provider could not be reached.
    Unreachable(ProviderUnreachable),
}

impl fmt::Display for ProviderFailure {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Status(status) => write!(out, "Cloudflare TURN credential request answered {status}"),
            Self::NoRelayEntry => out.write_str("Cloudflare TURN response carried no usable relay entry"),
            Self::Unreachable(inner) => fmt::Display::fmt(inner, out),
        }
    }
}

impl std::error::Error for ProviderFailure {}

impl From<ProviderUnreachable> for ProviderFailure {
    fn from(inner: ProviderUnreachable) -> Self {
        Self::Unreachable(inner)
    }
}

/// Asks Cloudflare for a TURN credential for `account`, valid for `ttl_seconds`.
pub async fn mint_cloudflare(
    cloudflare: &CloudflareTurn,
    account: &AccountId,
    ttl_seconds: u32,
    now: UnixMillis,
    http: &dyn ProviderHttp,
) -> Result<IssuedCredential, ProviderFailure> {
    let endpoint = format!(
        "{CLOUDFLARE_RTC_API}/turn/keys/{}/credentials/generate-ice-servers",
        cloudflare.key_id
    );
    let post = |tagged: bool| {
        let mut body = JsonObject::new().number("ttl", ttl_seconds);
        if tagged {
            body = body.string("customIdentifier", &relay_account_tag(account));
        }
        http.post(ProviderRequest {
            url: endpoint.clone(),
            bearer: cloudflare.api_token.clone(),
            body: body.finish(),
            timeout_ms: PROVIDER_TIMEOUT_MS,
        })
    };
    let mut answer = post(true).await?;
    // A key without custom identifiers refuses the tagged request; the untagged one still issues.
    if answer.status == 400 {
        answer = post(false).await?;
    }
    if !matches!(answer.status, 200..=299) {
        return Err(ProviderFailure::Status(answer.status));
    }
    let ProviderIceServers { stun_urls, turn } = normalize_provider_ice_servers(answer.json.as_ref());
    let Some((urls, username, credential)) = turn else {
        return Err(ProviderFailure::NoRelayEntry);
    };
    let credential = RelayCredential {
        expires_at: after(now, ttl_seconds),
        username,
        credential,
        urls,
    };
    Ok(IssuedCredential { stun_urls, credential })
}

/// Issues credentials through Cloudflare's TURN service.
#[derive(Clone)]
pub struct CloudflareIssuer {
    /// The Cloudflare TURN key.
    pub key: CloudflareTurn,
    /// HTTPS to Cloudflare.
    pub http: Arc<dyn ProviderHttp>,
}

impl CredentialIssuer for CloudflareIssuer {
    fn provider(&self) -> TurnProvider {
        TurnProvider::Cloudflare
    }

    fn issue(&self, to: AccountId, ttl_seconds: u32, at: UnixMillis) -> Answer<'_, IssuedCredential> {
        Box::pin(async move {
            let minted = mint_cloudflare(&self.key, &to, ttl_seconds, at, &*self.http).await;
            Ok(minted?)
        })
    }
}

/// The ICE servers read from a provider's answer.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProviderIceServers {
    /// Its STUN URLs.
    pub stun_urls: Vec<String>,
    /// TURN URLs, username and credential of the first usable TURN entry.
    pub turn: Option<(Vec<String>, String, Secret)>,
}

/// A TURN username or credential: 1 to 512 visible ASCII characters, no space.
fn turn_secret_text(value: Option<&Value>) -> Option<String> {
    let text = value.and_then(Value::as_str)?;
    let visible = text.chars().all(|c| c.is_ascii_graphic());
    (visible && matches!(text.len(), 1..=512)).then(|| text.to_owned())
}

/// The URLs of one provider entry, one or a list; port 53 is DNS, which most networks refuse and firewalls misread.
fn entry_urls(entry: &Value) -> Vec<&str> {
    let listed: Vec<&str> = match entry.get("urls") {
        Some(Value::Array(items)) => items.iter().filter_map(Value::as_str).collect(),
        Some(Value::String(only)) => vec![only.as_str()],
        _ => Vec::new(),
    };
    listed
        .into_iter()
        .filter(|url| !(url.ends_with(":53") || url.contains(":53?")))
        .collect()
}

/// Reads the provider's `iceServers` (one entry or a list): its STUN URLs and first usable TURN entry, dropping port 53.
pub fn normalize_provider_ice_servers(body: Option<&Value>) -> ProviderIceServers {
    let entries: &[Value] = match body.and_then(|body| body.get("iceServers")) {
        Some(Value::Array(items)) => items,
        Some(Value::Null) | None => &[],
        Some(single) => std::slice::from_ref(single),
    };
    let mut found = ProviderIceServers::default();
    for entry in entries {
        let urls = entry_urls(entry);
        for &url in &urls {
            if is_stun_url(url) && !found.stun_urls.iter().any(|seen| seen == url) {
                found.stun_urls.push(url.to_owned());
            }
        }
        if found.turn.is_some() {
            continue;
        }
        let relays: Vec<String> = urls
            .into_iter()
            .filter(|url| is_turn_url(url))
            .map(str::to_owned)
            .collect();
        let login = turn_secret_text(entry.get("username")).zip(turn_secret_text(entry.get("credential")));
        if let (false, Some((username, credential))) = (relays.is_empty(), login) {
            found.turn = Some((relays, username, Secret::new(credential)));
        }
    }
    found
}

/// The month so far, from `now`: (first day, today, month), as Cloudflare's analytics take them.
pub fn month_window(now: UnixMillis) -> (String, String, String) {
    let stamp = wire::iso_time(now);
    let (month, today) = (&stamp[..7], &stamp[..10]);
    (format!("{month}-01"), today.to_owned(), month.to_owned())
}

/// The egress bytes of one usage group; an unreadable figure counts as none.
fn group_bytes(group: &Value) -> f64 {
    let bytes = group
        .get("sum")
        .and_then(|sum| sum.get("egressBytes"))
        .map_or(f64::NAN, wire::js_number_value);
    if bytes.is_nan() { 0.0 } else { bytes }
}

/// The relay bytes this account's tag has used this month, or `None` when the figure cannot be read.
pub async fn read_monthly_relay_egress_bytes(
    analytics: &CloudflareAnalytics,
    account: &AccountId,
    now: UnixMillis,
    http: &dyn ProviderHttp,
) -> Option<f64> {
    let (from, to, _) = month_window(now);
    let [account_tag, date_from, date_to, identifier] =
        [&analytics.account_id, &from, &to, &relay_account_tag(account)].map(|text| wire::json_string(text));
    let query = format!(
        "query {{ viewer {{ usage: accounts(filter: {{ accountTag: {account_tag} }}) {{\n        \
         callsTurnUsageAdaptiveGroups(filter: {{ date_geq: {date_from}, date_leq: {date_to},\n            \
         customIdentifier: {identifier} }}, limit: 1, orderBy: [customIdentifier_ASC]) {{ sum {{ egressBytes }} }} }} }} }}"
    );
    let answer = http
        .post(ProviderRequest {
            body: JsonObject::new().string("query", &query).finish(),
            bearer: analytics.api_token.clone(),
            url: CLOUDFLARE_GRAPHQL_API.to_owned(),
            timeout_ms: PROVIDER_TIMEOUT_MS,
        })
        .await
        .ok()
        .filter(|answer| answer.status == 200)?;
    let json = answer.json?;
    let errors = json.get("errors").and_then(Value::as_array).map_or(0, Vec::len);
    if errors > 0 {
        return None;
    }
    let viewer = json.get("data")?.get("viewer")?;
    let usage = viewer
        .get("usage")
        .filter(|usage| !usage.is_null())
        .or_else(|| viewer.get("accounts"))?;
    let groups = usage.get(0)?.get("callsTurnUsageAdaptiveGroups")?.as_array()?;
    let total = groups.iter().map(group_bytes).sum::<f64>();
    (total >= 0.0 && total.is_finite()).then_some(total)
}

/// Monthly usage per account, kept ten minutes so a burst of configs asks Cloudflare once.
#[derive(Debug, Default)]
pub struct RelayUsageCache {
    entries: Mutex<HashMap<String, (String, f64, UnixMillis)>>,
}

impl RelayUsageCache {
    /// The account's relay bytes for `month`, while the cached figure is still fresh.
    pub fn get(&self, account: &AccountId, month: &str, now: UnixMillis) -> Option<f64> {
        let entries = guard(&self.entries);
        let &(ref for_month, bytes, read_at) = entries.get(&account.0)?;
        let fresh = now.0 - read_at.0 < USAGE_FRESH_MS;
        (fresh && for_month == month).then_some(bytes)
    }

    /// Caches the account's relay bytes for `month`, read at `now`.
    pub fn set(&self, account: &AccountId, month: &str, bytes: f64, now: UnixMillis) {
        let mut entries = guard(&self.entries);
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
    /// Counts one call for `key` and returns whether it is within `limit` calls in the last hour; a refused call is not counted.
    pub fn allow(&self, key: &str, limit: u32, now: UnixMillis) -> bool {
        let within = |at: &i64| now.0 - *at < WINDOW_MS;
        let mut hits = guard(&self.hits);
        let times = hits.entry(key.to_owned()).or_default();
        times.retain(within);
        if times.len() >= limit as usize {
            return false;
        }
        times.push(now.0);
        if hits.len() > MAX_TRACKED {
            hits.retain(|_, times| times.iter().any(within));
        }
        true
    }
}

wire_enum! {
    /// Why a transport gets no TURN relay; each travels as its code.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    RelayDenied {
        NotConfigured = "RELAY_NOT_CONFIGURED",
        QuotaExceeded = "RELAY_QUOTA_EXCEEDED",
        ProviderError = "RELAY_PROVIDER_ERROR",
    }
}

/// Whether a transport may use the TURN relay.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RelayStatus {
    /// Granted.
    Allowed {
        /// How long the credential lasts, in seconds.
        ttl_seconds: u32,
        /// When it stops working.
        expires_at: UnixMillis,
    },
    /// Not granted; the transport gets STUN only.
    Denied(RelayDenied),
}

/// One ICE server entry, as the transports read it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IceServer {
    /// Its STUN or TURN URLs.
    pub urls: Vec<String>,
    /// The TURN username; `None` for STUN.
    pub username: Option<String>,
    /// The TURN password; `None` for STUN.
    pub credential: Option<Secret>,
}

impl IceServer {
    /// A STUN entry.
    pub fn stun(urls: Vec<String>) -> Self {
        Self {
            username: None,
            credential: None,
            urls,
        }
    }

    /// A TURN entry for this credential.
    pub fn turn(credential: &RelayCredential) -> Self {
        let RelayCredential {
            username,
            credential,
            urls,
            ..
        } = credential.clone();
        Self {
            username: Some(username),
            credential: Some(credential),
            urls,
        }
    }

    /// The only place a relay credential is written out.
    pub fn to_json(&self) -> String {
        let secret = self.credential.as_ref().map(Secret::expose);
        JsonObject::new()
            .strings("urls", &self.urls)
            .optional_string("username", self.username.as_deref())
            .optional_string("credential", secret)
            .finish()
    }
}

/// The ICE servers and relay status for one transport.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelayGrant {
    /// STUN first, then the TURN entry when granted.
    pub ice_servers: Vec<IceServer>,
    /// Whether the relay was granted.
    pub relay: RelayStatus,
}

impl RelayGrant {
    /// The ICE servers as a JSON array.
    pub fn ice_servers_json(&self) -> String {
        wire::json_array(self.ice_servers.iter().map(IceServer::to_json))
    }

    /// The relay status as JSON: `allowed`, and the TTL and expiry or the reason.
    pub fn relay_json(&self) -> String {
        let body = JsonObject::new();
        let filled = match self.relay {
            RelayStatus::Denied(why) => body.boolean("allowed", false).string("reason", why.as_str()),
            RelayStatus::Allowed {
                ttl_seconds,
                expires_at,
            } => body
                .boolean("allowed", true)
                .number("ttlSeconds", ttl_seconds)
                .string("expiresAt", &wire::iso_time(expires_at)),
        };
        filled.finish()
    }

    /// `ISSUED`, or the refusal's code, for the log.
    pub fn result_label(&self) -> &'static str {
        let RelayStatus::Denied(why) = self.relay else {
            return "ISSUED";
        };
        why.as_str()
    }
}

/// Grants relay credentials under the server's relay settings.
#[derive(Clone)]
pub struct RelayGrants {
    /// The relay settings.
    pub config: Arc<RelayConfig>,
    /// HTTPS to the provider.
    pub http: Arc<dyn ProviderHttp>,
    /// Cached monthly relay usage per account.
    pub usage: Arc<RelayUsageCache>,
}

impl RelayGrants {
    /// The issuer for the configured provider; `None` when no provider is configured or its settings are missing.
    pub fn issuer(&self) -> Option<Box<dyn CredentialIssuer>> {
        let config = &*self.config;
        let issuer: Box<dyn CredentialIssuer> = match config.provider {
            TurnProvider::Cloudflare => Box::new(CloudflareIssuer {
                key: config.cloudflare.clone()?,
                http: Arc::clone(&self.http),
            }),
            TurnProvider::TurnRest => Box::new(TurnRestIssuer(config.turn_rest.clone()?)),
            TurnProvider::None => return None,
        };
        Some(issuer)
    }

    fn stun_only(&self, why: RelayDenied) -> RelayGrant {
        RelayGrant {
            relay: RelayStatus::Denied(why),
            ice_servers: vec![IceServer::stun(self.config.stun_urls.clone())],
        }
    }

    /// Whether the account has used its monthly relay quota, when the quota is measured at all. An unreadable
    /// figure does not block.
    async fn over_quota(&self, account: &AccountId, now: UnixMillis) -> bool {
        let config = &*self.config;
        let measured = config.provider == TurnProvider::Cloudflare && config.monthly_quota_bytes > 0;
        let Some(analytics) = config.analytics.as_ref().filter(|_| measured) else {
            return false;
        };
        let (_, _, month) = month_window(now);
        let used = match self.usage.get(account, &month, now) {
            Some(cached) => Some(cached),
            None => {
                let read = read_monthly_relay_egress_bytes(analytics, account, now, &*self.http).await;
                if let Some(bytes) = read {
                    self.usage.set(account, &month, bytes, now);
                }
                read
            }
        };
        used.is_some_and(|bytes| bytes >= config.monthly_quota_bytes as f64)
    }

    /// The ICE servers for one transport; STUN alone whenever the relay cannot be granted.
    pub async fn grant(&self, account: &AccountId, ttl_seconds: u32, now: UnixMillis) -> RelayGrant {
        let Some(issuer) = self.issuer() else {
            return self.stun_only(RelayDenied::NotConfigured);
        };
        if self.over_quota(account, now).await {
            return self.stun_only(RelayDenied::QuotaExceeded);
        }
        let Ok(issued) = issuer.issue(account.clone(), ttl_seconds, now).await else {
            return self.stun_only(RelayDenied::ProviderError);
        };
        // The configured STUN servers first, then the provider's new ones, each once.
        let stun_urls = issued
            .stun_urls
            .into_iter()
            .fold(self.config.stun_urls.clone(), |mut known, url| {
                if !known.contains(&url) {
                    known.push(url);
                }
                known
            });
        RelayGrant {
            relay: RelayStatus::Allowed {
                expires_at: after(now, ttl_seconds),
                ttl_seconds,
            },
            ice_servers: vec![IceServer::stun(stun_urls), IceServer::turn(&issued.credential)],
        }
    }
}

/// A credential that outlives the hunt by ten minutes, never under five minutes, never over the cap.
pub fn relay_ttl_seconds(hunt_expires_at: UnixMillis, now: UnixMillis, max_ttl_seconds: u32) -> u32 {
    let left_ms = hunt_expires_at.0.saturating_sub(now.0);
    // Division truncates toward zero, which rounds a negative remainder up already.
    let left_seconds = left_ms / 1000 + i64::from(left_ms % 1000 > 0);
    let wanted = left_seconds.saturating_add(600).max(300);
    u32::try_from(wanted.min(i64::from(max_ttl_seconds))).unwrap_or(max_ttl_seconds)
}

#[cfg(test)]
mod tests {
    use rustic_rifts_seam::poll_once;
    use serde_json::json;
    use sha1::Sha1;
    use sha2::Sha256;

    use super::*;

    type Posted = Result<ProviderAnswer, ProviderUnreachable>;

    /// Answers each POST from a queue and records the requests.
    #[derive(Default)]
    struct Recording {
        requests: Mutex<Vec<ProviderRequest>>,
        answers: Mutex<Vec<Posted>>,
    }

    impl Recording {
        fn answering(answers: Vec<Posted>) -> Arc<Self> {
            let recording = Self::default();
            *recording.answers.lock().unwrap() = answers;
            Arc::new(recording)
        }

        fn requests(&self) -> Vec<ProviderRequest> {
            self.requests.lock().unwrap().clone()
        }
    }

    impl ProviderHttp for Recording {
        fn post(&self, request: ProviderRequest) -> BoxFuture<'_, Posted> {
            self.requests.lock().unwrap().push(request);
            let mut queue = self.answers.lock().unwrap();
            let next = (!queue.is_empty()).then(|| queue.remove(0));
            Box::pin(ready(next.unwrap_or(Err(ProviderUnreachable))))
        }
    }

    fn ok(status: u16, json: Value) -> Posted {
        Ok(ProviderAnswer {
            json: Some(json),
            status,
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
            secret: Secret::new("0123456789abcdef".to_owned()),
            urls: vec!["turn:relay.example:3478".to_owned()],
        }
    }

    fn config(provider: TurnProvider) -> RelayConfig {
        let cloudflare = CloudflareTurn {
            api_token: Secret::new("cf-api-token".to_owned()),
            key_id: "abcdef0123456789".to_owned(),
        };
        RelayConfig {
            cloudflare: Some(cloudflare),
            turn_rest: Some(turn_rest()),
            provider,
            ..RelayConfig::default()
        }
    }

    fn grants(config: RelayConfig, http: Arc<Recording>) -> RelayGrants {
        RelayGrants {
            usage: Arc::default(),
            config: Arc::new(config),
            http,
        }
    }

    fn granted(grants: &RelayGrants, ttl_seconds: u32, now: i64) -> RelayGrant {
        poll_once(grants.grant(&account(), ttl_seconds, UnixMillis(now))).unwrap()
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
        assert_eq!(
            normalize_provider_ice_servers(Some(&single)).turn.unwrap().0,
            ["turn:one.example:3478"]
        );

        for empty in [None, Some(Value::Null), Some(json!({"iceServers": null}))] {
            assert_eq!(
                normalize_provider_ice_servers(empty.as_ref()),
                ProviderIceServers::default()
            );
        }
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
        let minted = mint_turn_rest(&turn_rest(), &account(), 600, UnixMillis(1_700_000_000_999));
        assert_eq!(minted.username, format!("1700000600:{}", relay_account_tag(&account())));
        assert_eq!(minted.credential.expose().len(), 28);
        assert_eq!(minted.urls, ["turn:relay.example:3478"]);
        assert_eq!(minted.expires_at, UnixMillis(1_700_000_600_999));

        let mac = <Hmac<Sha1>>::new_from_slice(b"0123456789abcdef")
            .unwrap()
            .chain_update(minted.username.as_bytes());
        assert_eq!(minted.credential.expose(), STANDARD.encode(mac.finalize().into_bytes()));
    }

    #[test]
    fn hmac_sha1_rfc_2202_case_2() {
        let mac = <Hmac<Sha1>>::new_from_slice(b"Jefe")
            .unwrap()
            .chain_update(b"what do ya want for nothing?");
        assert_eq!(
            hex::encode(mac.finalize().into_bytes()),
            "effcdf6ae5eb2fa2d27416d5f184df9c259a7c79"
        );
    }

    #[test]
    fn relay_tag_hides_the_account() {
        let tag = relay_account_tag(&account());
        assert_eq!(tag.len(), 32);
        assert!(tag.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')));
        assert_eq!(relay_account_tag(&account()), tag);
        assert_ne!(relay_account_tag(&AccountId("account-0002".to_owned())), tag);
        assert!(!tag.contains("account"));
        let full = hex::encode(Sha256::digest(b"rustic-relay-tag-v1\naccount-0001"));
        assert_eq!(tag, full[..32]);
    }

    #[test]
    fn stun_only_until_configured() {
        let http = Recording::answering(Vec::new());
        let unconfigured = grants(config(TurnProvider::None), Arc::clone(&http));
        let grant = granted(&unconfigured, 600, 0);
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
        let grant = granted(
            &grants(config(TurnProvider::Cloudflare), Arc::clone(&http)),
            4200,
            1_791_878_400_000,
        );
        let requests = http.requests();
        let url = "https://rtc.live.cloudflare.com/v1/turn/keys/abcdef0123456789/credentials/generate-ice-servers";
        let tag = relay_account_tag(&account());
        let bodies: Vec<&str> = requests.iter().map(|request| request.body.as_str()).collect();
        assert_eq!(
            bodies,
            [
                format!(r#"{{"ttl":4200,"customIdentifier":"{tag}"}}"#).as_str(),
                r#"{"ttl":4200}"#
            ]
        );
        for request in &requests {
            assert_eq!(
                (request.url.as_str(), request.bearer.expose(), request.timeout_ms),
                (url, "cf-api-token", 5000)
            );
            assert!(!request.body.contains("account-0001"));
        }
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
            let grant = granted(
                &grants(config(TurnProvider::Cloudflare), Recording::answering(answers)),
                600,
                0,
            );
            assert_eq!(
                (grant.result_label(), grant.ice_servers.len()),
                ("RELAY_PROVIDER_ERROR", 1)
            );
        }
        let key = config(TurnProvider::Cloudflare).cloudflare.unwrap();
        let mint = |answer: Posted| {
            let http = Recording::answering(vec![answer]);
            poll_once(mint_cloudflare(&key, &account(), 600, UnixMillis(0), http.as_ref())).unwrap()
        };
        assert_eq!(
            mint(ok(403, json!({}))).unwrap_err().to_string(),
            "Cloudflare TURN credential request answered 403"
        );
        assert_eq!(
            mint(ok(200, json!({"iceServers": []}))).unwrap_err(),
            ProviderFailure::NoRelayEntry
        );
        assert_eq!(ProviderUnreachable.to_string(), "relay provider request failed");
    }

    fn usage_answer(groups: Value) -> Posted {
        ok(
            200,
            json!({"data": {"viewer": {"usage": [{"callsTurnUsageAdaptiveGroups": groups}]}}}),
        )
    }

    fn analytics() -> CloudflareAnalytics {
        CloudflareAnalytics {
            api_token: Secret::new("analytics-token".to_owned()),
            account_id: "acct0123".to_owned(),
        }
    }

    fn usage_read(answer: Posted, now: i64) -> (Option<f64>, Vec<ProviderRequest>) {
        let http = Recording::answering(vec![answer]);
        let read = poll_once(read_monthly_relay_egress_bytes(
            &analytics(),
            &account(),
            UnixMillis(now),
            http.as_ref(),
        ));
        (read.unwrap(), http.requests())
    }

    #[test]
    fn usage_query_text() {
        let (read, requests) = usage_read(usage_answer(json!([])), 1_791_878_400_000);
        assert_eq!(read, Some(0.0));
        let request = &requests[0];
        assert_eq!(
            (request.url.as_str(), request.bearer.expose(), request.timeout_ms),
            (CLOUDFLARE_GRAPHQL_API, "analytics-token", 5000)
        );
        let tag = relay_account_tag(&account());
        let query = format!(
            "query {{ viewer {{ usage: accounts(filter: {{ accountTag: \"acct0123\" }}) {{\n        callsTurnUsageAdaptiveGroups(filter: {{ date_geq: \"2026-10-01\", date_leq: \"2026-10-13\",\n            customIdentifier: \"{tag}\" }}, limit: 1, orderBy: [customIdentifier_ASC]) {{ sum {{ egressBytes }} }} }} }} }}"
        );
        assert_eq!(request.body, format!("{{\"query\":{}}}", wire::json_string(&query)));
        assert!(request.body.contains("{\\n        callsTurn"));
        assert!(!request.body.contains("account-0001"));
    }

    #[test]
    fn usage_sums() {
        let read = |answer| usage_read(answer, 0).0;
        let accounts = ok(
            200,
            json!({"data": {"viewer": {"usage": null,
            "accounts": [{"callsTurnUsageAdaptiveGroups": [{"sum": {"egressBytes": 9}}]}]}}}),
        );
        let cases = [
            (
                usage_answer(json!([{"sum": {"egressBytes": "5"}}, {"sum": {"egressBytes": 7}}])),
                Some(12.0),
            ),
            (
                usage_answer(json!([{"sum": {"egressBytes": "x"}}, {"sum": {}}])),
                Some(0.0),
            ),
            (usage_answer(json!([{"sum": {"egressBytes": -3}}])), None),
            (usage_answer(json!({})), None),
            (ok(500, json!({})), None),
            (Err(ProviderUnreachable), None),
            (ok(200, json!({"errors": [{"message": "no"}], "data": null})), None),
            (accounts, Some(9.0)),
        ];
        for (index, (answer, expected)) in cases.into_iter().enumerate() {
            assert_eq!(read(answer), expected, "case {index}");
        }
    }

    #[test]
    fn quota_from_analytics_is_cached() {
        let quota_config = RelayConfig {
            analytics: Some(analytics()),
            monthly_quota_bytes: 1000,
            ..config(TurnProvider::Cloudflare)
        };
        let http = Recording::answering(vec![usage_answer(json!([{"sum": {"egressBytes": 1000}}]))]);
        let grants = grants(quota_config, Arc::clone(&http));
        for now in [0, 599_999] {
            assert_eq!(
                granted(&grants, 600, now).relay,
                RelayStatus::Denied(RelayDenied::QuotaExceeded)
            );
        }
        assert_eq!(http.requests().len(), 1);
        // The cache has lapsed and the analytics answer is gone: an unreadable figure does not block.
        let grants = RelayGrants {
            http: Recording::answering(vec![Err(ProviderUnreachable), ok(200, cloudflare_answer())]),
            ..grants
        };
        assert_eq!(granted(&grants, 600, 600_000).result_label(), "ISSUED");
    }

    #[test]
    fn usage_cache_and_hourly_counter() {
        let cache = RelayUsageCache::default();
        cache.set(&account(), "2026-10", 5.0, UnixMillis(0));
        let cached = |month: &str, now: i64| cache.get(&account(), month, UnixMillis(now));
        assert_eq!(
            [
                cached("2026-10", 599_999),
                cached("2026-10", 600_000),
                cached("2026-11", 1)
            ],
            [Some(5.0), None, None]
        );

        let counter = HourlyCounter::default();
        let steps = [
            ("k", 0, true),
            ("k", 1, true),
            ("k", 2, false),
            ("other", 2, true),
            ("k", 3_600_000, true),
            ("k", 3_600_000, false),
            ("k", 3_600_001, true),
        ];
        for (key, now, allowed) in steps {
            assert_eq!(counter.allow(key, 2, UnixMillis(now)), allowed, "{key} at {now}");
        }
    }

    #[test]
    fn month_window_and_ttls() {
        assert_eq!(
            month_window(UnixMillis(1_791_878_400_000)),
            ("2026-10-01".to_owned(), "2026-10-13".to_owned(), "2026-10".to_owned())
        );
        let now = UnixMillis(1_000_000_000);
        let ttl = |left_ms: i64| relay_ttl_seconds(UnixMillis(now.0 + left_ms), now, 10_800);
        assert_eq!(
            [ttl(3_600_000), ttl(-3_600_000), ttl(1500), ttl(86_400_000)],
            [4200, 300, 602, 10_800]
        );
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
        let grant = granted(&grants(config(TurnProvider::TurnRest), Arc::clone(&http)), 600, 0);
        assert!(http.requests().is_empty());
        assert_eq!(grant.ice_servers.len(), 2);
        let turn = &grant.ice_servers[1];
        assert_eq!(turn.urls, ["turn:relay.example:3478"]);
        assert!(turn.to_json().contains("\"credential\":\""));
        assert!(!format!("{grant:?}").contains(turn.credential.as_ref().unwrap().expose()));
    }
}
