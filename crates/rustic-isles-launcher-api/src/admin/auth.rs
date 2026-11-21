// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Admin authentication (admin spec §3.2 to §3.5): the session cookie with its CSRF token, and the manager operator
//! credential `crates/dainty-admin` and the dashboard sign every `/admin/v1` call with. Trust decisions read the system
//! clock, never the replay clock (admin decisions, item 1).

use std::net::IpAddr;
use std::sync::{Arc, LazyLock, Weak};

use axum::response::Response;
use dainty_module_api::{Actor, ActorContext, BoxFuture};
use http::{HeaderMap, Method, StatusCode};
use regex::Regex;
use rustic_isles::{AdminRequestHead, Shared};
use rustic_isles_store::IslesStore;

use super::{AdminCaller, AdminRequest, BodyFacts, LauncherApiAdmin, Via, admin_error, admin_internal};
use crate::launcher_api::is_account_eligible;
use crate::util::{self, hmac_sha256, iso, sha256_hex, system_now_ms};

/// The admin session cookie's name.
pub const SESSION_COOKIE: &str = "rustic_isles_admin";
/// How long (ms) a used operator signature is remembered, so a replay is refused.
pub const NONCE_TTL_MS: i64 = 120_000;
const OPERATOR_PREFIX: &str = "x-paradox-operator-";
const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

static ID: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"^[0-9]{1,32}$").ok());
static TIMESTAMP: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"^[0-9]{1,15}$").ok());
static SIGNATURE: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"^[0-9a-fA-F]{64}$").ok());

fn matches(re: &LazyLock<Option<Regex>>, text: &str) -> bool {
    re.as_ref().is_some_and(|r| r.is_match(text))
}

/// A cookie value that does not `decodeURIComponent`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UndecodableCookie;

/// The first part named `name` of the `Cookie` header, its value `decodeURIComponent`d.
pub fn read_cookie(headers: &HeaderMap, name: &str) -> Result<Option<String>, UndecodableCookie> {
    let Some(text) = util::header_text(headers, "cookie") else {
        return Ok(None);
    };
    for part in text.split(';') {
        let part = part.trim();
        let (key, value) = part.split_once('=').unwrap_or((part, ""));
        if key == name {
            return dainty_service_kit::js::decode_uri_component(value)
                .map(Some)
                .ok_or(UndecodableCookie);
        }
    }
    Ok(None)
}

/// `::1`, or dotted `127.x.x.x` (each part 1 to 3 digits), optionally `::ffff:`-prefixed.
pub fn is_loopback_peer(text: &str) -> bool {
    if text == "::1" {
        return true;
    }
    let v4 = text.strip_prefix("::ffff:").unwrap_or(text);
    let parts: Vec<&str> = v4.split('.').collect();
    parts.len() == 4
        && parts[0] == "127"
        && parts
            .iter()
            .all(|p| (1..=3).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_digit()))
}

fn has_forwarding(headers: &HeaderMap) -> bool {
    headers.keys().any(|name| {
        let n = name.as_str();
        n.starts_with("x-forwarded-") || matches!(n, "x-real-ip" | "forwarded" | "via")
    })
}

/// Whether the request carries any `x-paradox-operator-*` header.
pub fn has_operator_headers(headers: &HeaderMap) -> bool {
    headers.keys().any(|name| name.as_str().starts_with(OPERATOR_PREFIX))
}

/// Whether the manager's operator credential is accepted (`RUSTIC_ISLES_MANAGER_ADMIN=on`).
pub fn manager_admin_on(settings: &rustic_isles::Settings) -> bool {
    settings
        .raw("RUSTIC_ISLES_MANAGER_ADMIN")
        .is_some_and(|v| v.trim().to_lowercase() == "on")
}

/// The operator secret: JS-trimmed, at least 32 UTF-16 units.
pub fn operator_secret(settings: &rustic_isles::Settings) -> Option<String> {
    let secret = util::trim(settings.raw("RUSTIC_ISLES_MANAGER_OPERATOR_SECRET").unwrap_or(""));
    (util::utf16_len(secret) >= 32).then(|| secret.to_owned())
}

/// The five lines the operator signs.
pub fn canonical_string(method: &str, target: &str, timestamp: &str, operator_id: &str, body: &[u8]) -> String {
    let digest = if body.is_empty() {
        EMPTY_SHA256.to_owned()
    } else {
        sha256_hex(body)
    };
    [method.to_uppercase().as_str(), target, timestamp, operator_id, &digest].join("\n")
}

/// The lower-case hex HMAC-SHA256 of the canonical string under the secret.
pub fn operator_signature(secret: &str, method: &str, target: &str, timestamp: &str, id: &str, body: &[u8]) -> String {
    hex::encode(hmac_sha256(
        secret.as_bytes(),
        canonical_string(method, target, timestamp, id, body).as_bytes(),
    ))
}

/// What the operator verifier reads (a pure function of these, the settings and the nonce set).
pub struct OperatorRequest<'a> {
    /// The HTTP method.
    pub method: &'a str,
    /// Path and query as received; part of what is signed.
    pub target: &'a str,
    /// The socket peer; only loopback is accepted.
    pub peer: Option<IpAddr>,
    /// The request's headers.
    pub headers: &'a HeaderMap,
    /// The body's bytes, whose digest is signed.
    pub body: &'a [u8],
    /// Whether the request declares a body.
    pub body_declared: bool,
    /// Whether the body was read as JSON.
    pub json_read: bool,
    /// The system clock, Unix ms.
    pub now_ms: i64,
}

/// The operator's verdict: accepted (id, display name) or refused (status, code, message).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperatorVerdict {
    /// The signature checks out.
    Accepted {
        /// The operator id.
        id: String,
        /// The operator's display name.
        name: String,
    },
    /// Refused with the status, error code and message.
    Refused(u16, &'static str, &'static str),
}

fn single_header(headers: &HeaderMap, name: &str) -> Result<Option<String>, ()> {
    let mut values = headers.get_all(name).iter();
    let Some(first) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() {
        return Err(());
    }
    first.to_str().map(|v| Some(v.trim().to_owned())).map_err(|_| ())
}

fn display_name(raw: Option<&str>) -> String {
    let cleaned: String = raw
        .unwrap_or("")
        .chars()
        .filter(|c| !matches!(c, '\u{0}'..='\u{1f}' | '\u{7f}'))
        .collect();
    let cut = util::take_utf16(&cleaned, 64);
    let name = util::trim(&cut);
    if name.is_empty() {
        "operator".to_owned()
    } else {
        name.to_owned()
    }
}

/// §3.3's checks, in order.
pub fn verify_operator(
    secret: Option<&str>,
    admin: &LauncherApiAdmin,
    request: &OperatorRequest<'_>,
) -> OperatorVerdict {
    use OperatorVerdict::Refused;
    let peer = request.peer.map(|p| p.to_string()).unwrap_or_default();
    if !is_loopback_peer(&peer) {
        return Refused(
            403,
            "ADMIN_OPERATOR_PEER_REFUSED",
            "Operator requests are accepted only from the loopback interface.",
        );
    }
    if has_forwarding(request.headers) {
        return Refused(
            403,
            "ADMIN_OPERATOR_FORWARDED",
            "Operator requests must not pass through a forwarding proxy.",
        );
    }
    let malformed = Refused(401, "ADMIN_OPERATOR_INVALID", "The operator credential is malformed.");
    let read = |name: &str| single_header(request.headers, &format!("{OPERATOR_PREFIX}{name}"));
    let (Ok(Some(id)), Ok(Some(timestamp)), Ok(Some(signature)), Ok(name)) =
        (read("id"), read("timestamp"), read("signature"), read("name"))
    else {
        return malformed;
    };
    if !matches(&ID, &id) || !matches(&TIMESTAMP, &timestamp) || !matches(&SIGNATURE, &signature) {
        return malformed;
    }
    let Some(secret) = secret else {
        return Refused(
            503,
            "ADMIN_OPERATOR_NOT_CONFIGURED",
            "The operator credential is not configured on this server.",
        );
    };
    let sent: i64 = timestamp.parse().unwrap_or(i64::MAX);
    if request.now_ms.saturating_sub(sent) > 60_000 || sent.saturating_sub(request.now_ms) > 5_000 {
        return Refused(
            401,
            "ADMIN_OPERATOR_STALE",
            "The operator request timestamp is outside the accepted window.",
        );
    }
    if request.body_declared && !request.json_read {
        return Refused(
            400,
            "ADMIN_OPERATOR_BODY",
            "An operator request with a body must send Content-Type application/json.",
        );
    }
    let expected = operator_signature(secret, request.method, request.target, &timestamp, &id, request.body);
    let signature = signature.to_lowercase();
    if !util::ct_eq(expected.as_bytes(), signature.as_bytes()) {
        return Refused(
            401,
            "ADMIN_OPERATOR_SIGNATURE",
            "The operator signature does not match the request.",
        );
    }
    if !admin.accept_nonce(&signature, request.now_ms) {
        return Refused(
            401,
            "ADMIN_OPERATOR_REPLAY",
            "This operator request was already accepted.",
        );
    }
    OperatorVerdict::Accepted {
        id,
        name: display_name(name.as_deref()),
    }
}

fn manager_caller(id: &str, name: String, now_ms: i64) -> AdminCaller {
    AdminCaller {
        user_id: format!("manager:{id}"),
        display_name: name,
        account: None,
        session: None,
        via: Via::Manager,
        created_at: iso(now_ms),
    }
}

/// The parts of a request authentication reads.
pub struct Presented<'a> {
    /// The HTTP method.
    pub method: &'a Method,
    /// Path and query as received.
    pub target: &'a str,
    /// The path alone.
    pub path: &'a str,
    /// The socket peer.
    pub peer: Option<IpAddr>,
    /// The request's headers.
    pub headers: &'a HeaderMap,
    /// The body's bytes.
    pub body: &'a [u8],
    /// What the body declared and how it was read.
    pub facts: &'a BodyFacts,
}

async fn authenticate(shared: &Shared, store: &IslesStore, presented: Presented<'_>) -> Result<AdminCaller, Response> {
    let settings = shared.settings();
    let now = system_now_ms();
    if has_operator_headers(presented.headers) && manager_admin_on(settings) {
        let admin = LauncherApiAdmin::of(shared);
        let verdict = verify_operator(
            operator_secret(settings).as_deref(),
            &admin,
            &OperatorRequest {
                method: presented.method.as_str(),
                target: presented.target,
                peer: presented.peer,
                headers: presented.headers,
                body: presented.body,
                body_declared: presented.facts.declared,
                json_read: presented.facts.json_read,
                now_ms: now,
            },
        );
        return match verdict {
            OperatorVerdict::Accepted { id, name } => Ok(manager_caller(&id, name, now)),
            OperatorVerdict::Refused(status, code, message) => {
                tracing::warn!(
                    "[ADMIN] manager operator request refused: {code} {} {}",
                    presented.method,
                    presented.path
                );
                Err(admin_error(
                    StatusCode::from_u16(status).unwrap_or(StatusCode::UNAUTHORIZED),
                    code,
                    message,
                ))
            }
        };
    }
    session_caller(store, presented.method, presented.headers, now).await
}

async fn session_caller(
    store: &IslesStore,
    method: &Method,
    headers: &HeaderMap,
    now: i64,
) -> Result<AdminCaller, Response> {
    let unauthorized = |message: &str| admin_error(StatusCode::UNAUTHORIZED, "ADMIN_UNAUTHORIZED", message);
    let token = match read_cookie(headers, SESSION_COOKIE) {
        Err(UndecodableCookie) => return Err(util::error_page(StatusCode::INTERNAL_SERVER_ERROR)),
        Ok(None) => return Err(unauthorized("Administrator sign-in required.")),
        Ok(Some(token)) if token.is_empty() => return Err(unauthorized("Administrator sign-in required.")),
        Ok(Some(token)) => token,
    };
    let hash = sha256_hex(token.as_bytes());
    let now_iso = iso(now);
    let found = store
        .read(move |conn, repos| {
            let Some(session) = repos.admin.find_active_session_by_token_hash(conn, &hash, &now_iso)? else {
                return Ok(None);
            };
            let account = repos.launcher_accounts.find_by_account(conn, &session.account)?;
            let user_id = super::user_id_of(conn, repos, &session.account);
            Ok(Some((session, account, user_id)))
        })
        .await
        .map_err(admin_internal)?;
    let Some((session, account, user_id)) = found else {
        return Err(unauthorized("Administrator session expired."));
    };
    let admitted = account
        .as_ref()
        .filter(|a| a.roles.iter().any(|r| r == "admin") && is_account_eligible(a));
    let Some(account) = admitted.cloned() else {
        let id = session.id.clone();
        let at = iso(now);
        let _ = store
            .write(&ActorContext::new(Actor::System), move |tx, repos| {
                repos.admin.revoke_session(tx, &id, &at)
            })
            .await;
        return Err(admin_error(
            StatusCode::FORBIDDEN,
            "ADMIN_FORBIDDEN",
            "Administrator access denied.",
        ));
    };
    if !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
        let sent = util::header_text(headers, "x-csrf-token").unwrap_or_default();
        if !util::ct_eq(sent.as_bytes(), session.csrf_token.as_bytes()) {
            return Err(admin_error(
                StatusCode::FORBIDDEN,
                "ADMIN_CSRF",
                "Invalid request token.",
            ));
        }
    }
    Ok(AdminCaller {
        user_id,
        display_name: account.display_name.clone(),
        account: Some(account),
        session: Some(session),
        via: Via::Session,
        created_at: String::new(),
    })
}

/// §3.3, else §3.2, for a request the wrapper read.
pub async fn authenticate_admin(
    shared: &Shared,
    store: &IslesStore,
    request: &AdminRequest,
    facts: &BodyFacts,
) -> Result<AdminCaller, Response> {
    authenticate(
        shared,
        store,
        Presented {
            method: &request.method,
            target: &request.target,
            path: &request.path,
            peer: request.peer,
            headers: &request.headers,
            body: &request.raw_body,
            facts,
        },
    )
    .await
}

/// Core's authenticator for the `/admin/` rifts rows (§3.5): the origin check, then the credential. A module with no
/// store refuses as an empty slot does.
pub struct AdminAuthenticatorSlot {
    shared: Weak<Shared>,
}

impl AdminAuthenticatorSlot {
    /// A slot over the module, held weakly.
    pub fn new(shared: &Arc<Shared>) -> Self {
        Self {
            shared: Arc::downgrade(shared),
        }
    }
}

impl rustic_isles::AdminAuthenticator for AdminAuthenticatorSlot {
    fn authenticate<'a>(
        &'a self,
        request: AdminRequestHead<'a>,
    ) -> BoxFuture<'a, Result<rustic_isles::AdminCaller, Response>> {
        Box::pin(async move {
            let Some(shared) = self.shared.upgrade() else {
                return Err(rustic_isles::admin_refusal());
            };
            let origin = super::check_origin(shared.settings(), request.headers)?;
            let Some(store) = util::store_of(&shared) else {
                return Err(rustic_isles::admin_refusal());
            };
            let method = Method::from_bytes(request.method.as_bytes()).unwrap_or(Method::GET);
            let facts = BodyFacts {
                declared: super::declared_body(request.headers),
                json_read: !request.body.is_empty() && super::is_json_type(request.headers),
            };
            let path = request.target.split('?').next().unwrap_or("");
            let presented = Presented {
                method: &method,
                target: request.target,
                path,
                peer: request.peer,
                headers: request.headers,
                body: request.body,
                facts: &facts,
            };
            match authenticate(&shared, &store, presented).await {
                Ok(caller) => Ok(rustic_isles::AdminCaller { key_id: caller.user_id }),
                Err(refused) => Err(super::admin_headers(refused, origin.as_deref())),
            }
        })
    }
}

/// The start-up check: with the manager switch on, the operator secret must be long enough.
pub fn check_operator_config(settings: &rustic_isles::Settings) -> Result<(), String> {
    if manager_admin_on(settings) && operator_secret(settings).is_none() {
        return Err(
            "RUSTIC_ISLES_MANAGER_OPERATOR_SECRET must contain at least 32 characters when \
                    RUSTIC_ISLES_MANAGER_ADMIN=on."
                .to_owned(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookies_and_peers() {
        let mut headers = HeaderMap::new();
        headers.insert("cookie", "a=1; rustic_isles_admin=x%20y; b".parse().unwrap());
        assert_eq!(read_cookie(&headers, SESSION_COOKIE), Ok(Some("x y".to_owned())));
        assert_eq!(read_cookie(&headers, "b"), Ok(Some(String::new())));
        assert_eq!(read_cookie(&headers, "mysticparadox_admin"), Ok(None));
        headers.insert("cookie", "rustic_isles_admin=%E0%A4%A".parse().unwrap());
        assert_eq!(read_cookie(&headers, SESSION_COOKIE), Err(UndecodableCookie));
        assert!(is_loopback_peer("127.0.0.1") && is_loopback_peer("::ffff:127.1.2.3") && is_loopback_peer("::1"));
        assert!(!is_loopback_peer("10.0.0.1") && !is_loopback_peer("127.0.0.1000") && !is_loopback_peer(""));
        assert_eq!(display_name(Some(" a\u{1}b ")), "ab");
        assert_eq!(display_name(None), "operator");
        assert!(
            check_operator_config(&rustic_isles::Settings::from_pairs([(
                "RUSTIC_ISLES_MANAGER_ADMIN",
                "on"
            )]))
            .is_err()
        );
    }
}
