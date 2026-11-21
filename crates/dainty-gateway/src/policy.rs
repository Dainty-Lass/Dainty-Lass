// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The route policy: what may reach a loopback service, decided from the method, the target and the header names
//! before any body is read. The 1.14.7 runtime wraps the game's calls as `/__origin/<host>/<path>`, which the
//! backend unwraps, so every rule judges the inner path.

use dainty_service_kit::js;
use http::HeaderMap;

/// The public `/dainty/api` routes by canonical name, with their methods; the rest of the API is the host's.
pub const PUBLIC_LAUNCHER_API: &[(&str, &[&str])] = &[
    ("register", &["POST"]),
    ("getuserinfo", &["GET", "HEAD"]),
    ("serverstatus", &["GET", "HEAD"]),
    ("registrationstatus", &["GET", "HEAD"]),
    ("playtime", &["GET", "HEAD"]),
    ("loginqueue", &["POST", "DELETE"]),
    ("gameticket", &["POST"]),
    ("oauthstart", &["POST"]),
    ("oauthfinish", &["POST"]),
];

/// The backend's admin and launcher APIs, metrics and probes: never from off this machine, but for
/// [`PUBLIC_REALM_PREFIXES`].
pub const HOST_ONLY_PREFIXES: &[&str] = &["/admin", "/launcher", "/internal", "/health", "/metrics"];

/// The only `/launcher` routes a launcher reaches from off this machine: realms, the P2P control plane and its
/// admission key. Node refused them all, so realms never worked through a gateway (docs/security/gateway.md).
pub const PUBLIC_REALM_PREFIXES: &[&str] = &["/launcher/v1/realms", "/launcher/v1/realm-hosting", "/launcher/v1/p2p"];

/// The methods the realm routes answer.
pub const PUBLIC_REALM_METHODS: &[&str] = &["GET", "POST"];

/// The header the launcher sends its account key (`DLK_...`) in.
pub const ACCOUNT_KEY_HEADER: &str = "x-dainty-user-api-key";

/// The game-server key header the backend reads first; host-only.
pub const GAMESERVER_KEY_HEADER: &str = "x-rustic-isles-gameserver-apikey";

/// The game-server key under each name the backend still reads (DX4).
pub const GAMESERVER_KEY_HEADERS: &[&str] = &[
    GAMESERVER_KEY_HEADER,
    "x-rustic-isles-gameserver-key",
    "x-mysticparadox-gameserver-apikey",
    "x-mysticparadox-gameserver-key",
];

/// Headers refused on any request from off this machine.
pub const HOST_ONLY_HEADERS: &[&str] = &[
    GAMESERVER_KEY_HEADER,
    "x-rustic-isles-gameserver-key",
    "x-mysticparadox-gameserver-apikey",
    "x-mysticparadox-gameserver-key",
    "x-dainty-admin-key",
    // The name before the rename, refused too so it never reaches a service
    "x-reforged-admin-key",
    "x-rustic-isles-metrics-token",
    "x-mysticparadox-metrics-token",
];

const ALLOWED_METHODS: &[&str] = &["GET", "HEAD", "POST", "PUT", "DELETE", "PATCH", "OPTIONS"];

/// A loopback service the gateway proxies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Upstream {
    /// The game backend.
    Backend,
    /// The content server.
    Content,
    /// The WebSocket listener the game's chat upgrades reach
    Ws,
}

impl Upstream {
    /// The access log's route.
    pub fn route(self) -> &'static str {
        match self {
            Self::Backend => "backend",
            Self::Content => "content",
            Self::Ws => "ws",
        }
    }
}

/// The rate-limit bucket a request spends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitClass {
    /// Most requests.
    General,
    /// `/content` downloads.
    Content,
    /// `POST /dainty/api/Register`.
    Register,
    /// `POST /account/api/oauth/token`.
    Token,
    /// WebSocket upgrades.
    Ws,
}

impl LimitClass {
    /// The class's name in logs and replies.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Content => "content",
            Self::Register => "register",
            Self::Token => "token",
            Self::Ws => "ws",
        }
    }
}

/// What a 2xx answer reports to the allowlist feed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedKind {
    /// Only when the request carried a bearer ([`has_bearer`])
    Heartbeat,
    /// A login token was issued.
    Token,
}

/// The routes the gateway answers itself (DN7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkRoute {
    /// `GET /network/v1/nodes`.
    Nodes,
    /// `GET /network/v1/ping`.
    Ping,
    /// `POST /network/v1/regions`.
    Regions,
}

impl NetworkRoute {
    const ALL: [(Self, &'static str, &'static str); 3] = [
        (Self::Nodes, "/network/v1/nodes", "GET"),
        (Self::Ping, "/network/v1/ping", "GET"),
        (Self::Regions, "/network/v1/regions", "POST"),
    ];

    /// The route's path.
    pub fn path(self) -> &'static str {
        Self::ALL.iter().find(|(r, ..)| *r == self).map_or("", |(_, p, _)| p)
    }

    /// The route's method.
    pub fn method(self) -> &'static str {
        Self::ALL.iter().find(|(r, ..)| *r == self).map_or("", |(.., m)| m)
    }
}

/// What the policy decided for one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Answered with `{"error":"<error>"}`; `reason` is for the access log
    Reject {
        /// HTTP status.
        status: u16,
        /// The error code in the body.
        error: &'static str,
        /// Why, for the access log.
        reason: &'static str,
        /// The bucket it spends.
        limit_class: LimitClass,
    },
    /// Proxied to a loopback service.
    Proxy {
        /// Where it goes.
        upstream: Upstream,
        /// The bucket it spends.
        limit_class: LimitClass,
        /// What a 2xx answer reports to the allowlist feed.
        feed: Option<FeedKind>,
    },
    /// Answered by the gateway itself.
    Network {
        /// The route that answers.
        route: NetworkRoute,
        /// The bucket it spends.
        limit_class: LimitClass,
    },
}

impl Decision {
    /// The bucket the request spends.
    pub fn class(&self) -> LimitClass {
        match self {
            Self::Reject { limit_class, .. } | Self::Proxy { limit_class, .. } | Self::Network { limit_class, .. } => {
                *limit_class
            }
        }
    }

    /// A refusal with `status` and `{"error":"<error>"}`.
    pub fn reject(status: u16, error: &'static str, reason: &'static str, class: LimitClass) -> Self {
        Self::Reject {
            status,
            error,
            reason,
            limit_class: class,
        }
    }
}

/// Origin form only: the path before the first `?` and the query after it.
pub fn split_target(raw: &str) -> Option<(&str, &str)> {
    if !raw.starts_with('/') {
        return None;
    }
    Some(raw.split_once('?').unwrap_or((raw, "")))
}

/// False for a path that could mean something else once a server normalises it.
pub fn check_path(path: &str) -> bool {
    if path
        .chars()
        .any(|c| c == '\\' || c <= '\u{1f}' || c == '\u{7f}' || js::is_space(c))
    {
        return false;
    }
    let lower = path.to_ascii_lowercase();
    if ["%2f", "%5c", "%00", "%2e"].iter().any(|bad| lower.contains(bad)) {
        return false;
    }
    !path.split('/').any(|segment| segment == "." || segment == "..")
}

/// Decoded as `decodeURIComponent`, lower-cased, slashes folded, no trailing slash but on `/`.
pub fn canonical_path(path: &str) -> Option<String> {
    let decoded = js::decode_uri_component(path)?.to_lowercase();
    let mut out = String::with_capacity(decoded.len());
    for c in decoded.chars() {
        if c == '/' && out.ends_with('/') {
            continue;
        }
        out.push(c);
    }
    if out.len() > 1 && out.ends_with('/') {
        out.pop();
    }
    Some(out)
}

/// `canonical` is `prefix` or continues with `/` after it.
pub fn under(canonical: &str, prefix: &str) -> bool {
    canonical
        .strip_prefix(prefix)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

/// A request path with its `/__origin/<host>` wrapper taken off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unwrapped {
    /// The inner path.
    pub path: String,
    /// The official host the runtime wrapped the call for
    pub origin: Option<String>,
}

fn is_wrapped(path: &str) -> Option<&str> {
    let rest = path.trim_start_matches('/');
    let head = rest.get(..8)?;
    let after = &rest[8..];
    (head.eq_ignore_ascii_case("__origin") && (after.is_empty() || after.starts_with('/'))).then_some(after)
}

fn is_origin_host(host: &str) -> bool {
    (1..=253).contains(&host.len())
        && host
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-')
        && !host.starts_with('.')
        && !host.ends_with('.')
        && !host.contains("..")
}

/// `/__origin/<host>/<path>` to `<path>` and the host; any other path as it is. `None` is a 400 `bad_origin`.
pub fn unwrap_origin(path: &str) -> Option<Unwrapped> {
    let Some(after) = is_wrapped(path) else {
        return Some(Unwrapped {
            path: path.to_owned(),
            origin: None,
        });
    };
    let rest = after.strip_prefix('/')?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (raw_host, inner) = rest.split_at(end);
    if raw_host.is_empty() {
        return None;
    }
    let host = js::decode_uri_component(raw_host)?.to_lowercase();
    if !is_origin_host(&host) {
        return None;
    }
    let inner = match inner {
        "" => "/".to_owned(),
        p if p.starts_with('/') => p.to_owned(),
        p => format!("/{p}"),
    };
    if is_wrapped(&inner).is_some() {
        return None;
    }
    Some(Unwrapped {
        path: inner,
        origin: Some(host),
    })
}

/// `Authorization` shaped as `/^bearer\s+\S+/i`. Only the shape: the backend never checks a heartbeat's token, so
/// any bearer opens the game ports for its sender (Node's behaviour, kept on purpose; docs/security/gateway.md).
pub fn has_bearer(headers: &HeaderMap) -> bool {
    let Some(value) = headers.get(http::header::AUTHORIZATION).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    let Some(scheme) = value.get(..6) else {
        return false;
    };
    if !scheme.eq_ignore_ascii_case("bearer") {
        return false;
    }
    let rest = &value[6..];
    let token = rest.trim_start_matches(js::is_space);
    token.len() < rest.len() && token.chars().next().is_some_and(|c| !js::is_space(c))
}

/// The game served is fixed (reborn), so this is [`classify`].
pub fn classify_for(
    _game: &dainty_protocol::GameVersion,
    method: &str,
    target: &str,
    headers: &HeaderMap,
    is_upgrade: bool,
) -> Decision {
    classify(method, target, headers, is_upgrade)
}

/// Decides where a request goes from its method, target, header names and whether it upgrades; reads no body.
pub fn classify(method: &str, target: &str, headers: &HeaderMap, is_upgrade: bool) -> Decision {
    use LimitClass::General;
    let method = method.to_ascii_uppercase();
    let Some((path, _)) = split_target(target) else {
        return Decision::reject(400, "bad_request", "target_not_origin_form", General);
    };
    if !check_path(path) {
        return Decision::reject(400, "bad_path", "bad_path", General);
    }
    let Some(unwrapped) = unwrap_origin(path) else {
        return Decision::reject(400, "bad_path", "bad_origin", General);
    };
    let Some(canonical) = canonical_path(&unwrapped.path) else {
        return Decision::reject(400, "bad_path", "bad_percent_encoding", General);
    };
    let wrapped = unwrapped.origin.is_some();
    let content = !wrapped && under(&canonical, "/content");
    let class = if is_upgrade {
        LimitClass::Ws
    } else if [
        "/dainty/api/register",
        "/dainty/api/oauthstart",
        "/dainty/api/oauthfinish",
    ]
    .contains(&canonical.as_str())
    {
        LimitClass::Register
    } else if canonical == "/account/api/oauth/token" {
        LimitClass::Token
    } else if content {
        LimitClass::Content
    } else {
        General
    };
    if !ALLOWED_METHODS.contains(&method.as_str()) {
        return Decision::reject(405, "method_not_allowed", "method", class);
    }
    if let Some(name) = HOST_ONLY_HEADERS.iter().find(|h| headers.contains_key(**h)) {
        let reason = if GAMESERVER_KEY_HEADERS.contains(name) {
            "gameserver_key_header"
        } else {
            "host_only_header"
        };
        return Decision::reject(403, "forbidden", reason, class);
    }
    if !wrapped && let Some((route, _, wanted)) = NetworkRoute::ALL.iter().find(|(_, p, _)| *p == canonical) {
        if is_upgrade {
            return Decision::reject(400, "bad_request", "unsupported_upgrade", class);
        }
        if method != *wanted {
            return Decision::reject(405, "method_not_allowed", "method", class);
        }
        return Decision::Network {
            route: *route,
            limit_class: class,
        };
    }
    if !wrapped && PUBLIC_REALM_PREFIXES.iter().any(|p| under(&canonical, p)) {
        return realm(&method, headers, is_upgrade, class);
    }
    // The launcher API's prefix before the 2026-11-06 rename: nothing serves it, and nothing reaches past here
    if under(&canonical, "/reforged") {
        return Decision::reject(403, "forbidden", "admin_route", class);
    }
    if under(&canonical, "/dainty") {
        let public = canonical
            .strip_prefix("/dainty/api/")
            .filter(|name| !name.is_empty() && !name.contains('/'))
            .and_then(|name| PUBLIC_LAUNCHER_API.iter().find(|(n, _)| *n == name))
            .is_some_and(|(_, methods)| !is_upgrade && methods.contains(&method.as_str()));
        if !public {
            return Decision::reject(403, "forbidden", "admin_route", class);
        }
        return Decision::Proxy {
            upstream: Upstream::Backend,
            limit_class: class,
            feed: None,
        };
    }
    if HOST_ONLY_PREFIXES.iter().any(|p| under(&canonical, p)) {
        return Decision::reject(403, "forbidden", "host_only_route", class);
    }
    if is_upgrade {
        let websocket = headers
            .get(http::header::UPGRADE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.eq_ignore_ascii_case("websocket"));
        if method != "GET" || !websocket {
            return Decision::reject(400, "bad_request", "unsupported_upgrade", class);
        }
        return Decision::Proxy {
            upstream: Upstream::Ws,
            limit_class: class,
            feed: None,
        };
    }
    if content {
        return Decision::Proxy {
            upstream: Upstream::Content,
            limit_class: class,
            feed: None,
        };
    }
    let feed = match (method.as_str(), canonical.as_str()) {
        ("POST", "/heartbeat") => Some(FeedKind::Heartbeat),
        ("POST", "/account/api/oauth/token") => Some(FeedKind::Token),
        _ => None,
    };
    Decision::Proxy {
        upstream: Upstream::Backend,
        limit_class: class,
        feed,
    }
}

/// A realm route: the account key and nothing else, one plausible value. A bearer would win over the key in the
/// backend's `launcher_auth`, and only the host-only auth routes issue one; the backend still checks the key.
fn realm(method: &str, headers: &HeaderMap, is_upgrade: bool, class: LimitClass) -> Decision {
    if is_upgrade {
        return Decision::reject(400, "bad_request", "unsupported_upgrade", class);
    }
    if !PUBLIC_REALM_METHODS.contains(&method) {
        return Decision::reject(405, "method_not_allowed", "method", class);
    }
    if headers.contains_key(http::header::AUTHORIZATION) {
        return Decision::reject(403, "forbidden", "realm_bearer", class);
    }
    let mut keys = headers.get_all(ACCOUNT_KEY_HEADER).iter();
    let key = keys.next().and_then(|v| v.to_str().ok());
    if keys.next().is_some() || !key.is_some_and(dainty_protocol::account::is_plausible_account_key) {
        return Decision::reject(401, "unauthorized", "no_account_key", class);
    }
    Decision::Proxy {
        upstream: Upstream::Backend,
        limit_class: class,
        feed: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::HeaderValue;

    fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.append(*name, HeaderValue::from_static(value));
        }
        map
    }

    fn decide(method: &str, target: &str) -> Decision {
        classify(method, target, &HeaderMap::new(), false)
    }

    fn proxied(d: &Decision) -> bool {
        matches!(d, Decision::Proxy { .. })
    }

    #[test]
    fn path_helpers() {
        assert_eq!(split_target("/a?b?c"), Some(("/a", "b?c")));
        assert_eq!(split_target("/a"), Some(("/a", "")));
        for bad in ["http://host/a", "*", ""] {
            assert_eq!(split_target(bad), None);
        }
        assert!(check_path("/a.b/c..d/.e"));
        for bad in [
            "/a\\b",
            "/a\u{1}",
            "/a\u{7f}",
            "/a b",
            "/a\u{a0}",
            "/a\u{feff}",
            "/a%2F",
            "/%5c",
            "/%00",
            "/%2E",
            "/./a",
            "/a/..",
        ] {
            assert!(!check_path(bad), "{bad:?}");
        }
        assert_eq!(
            canonical_path("/Dainty/API/Register/").as_deref(),
            Some("/dainty/api/register")
        );
        assert_eq!(canonical_path("//a///b//").as_deref(), Some("/a/b"));
        assert_eq!(canonical_path("/%41").as_deref(), Some("/a"));
        assert_eq!(canonical_path("/"), Some("/".into()));
        assert_eq!(canonical_path("/%zz"), None);
        assert_eq!(canonical_path("/%C3"), None);
        assert!(under("/admin", "/admin") && under("/admin/x", "/admin"));
        assert!(!under("/administrator", "/admin") && !under("/launchers", "/launcher"));
    }

    #[test]
    fn origin_unwrapping() {
        let ok = |p: &str| unwrap_origin(p).unwrap();
        assert_eq!(
            ok("/__origin/gamedev-prod.steelyard.ca/party"),
            Unwrapped {
                path: "/party".into(),
                origin: Some("gamedev-prod.steelyard.ca".into())
            }
        );
        assert_eq!(
            ok("/__origin/API.EpicGames.dev").origin.as_deref(),
            Some("api.epicgames.dev")
        );
        assert_eq!(ok("/__origin/API.EpicGames.dev").path, "/");
        let odd = ok("//__Origin/a%2Db.example/x");
        assert_eq!((odd.path.as_str(), odd.origin.as_deref()), ("/x", Some("a-b.example")));
        assert_eq!(ok("/__originx/y").origin, None);
        let long = format!("/__origin/{}", "a".repeat(254));
        for bad in [
            "/__origin",
            "/__origin/",
            long.as_str(),
            "/__origin/x/__origin/y/z",
            "/__origin/user@host/x",
            "/__origin/host:443/x",
            "/__origin/.a/x",
            "/__origin/a../x",
            "/__origin/%zz/x",
        ] {
            assert!(unwrap_origin(bad).is_none(), "{bad}");
        }
    }

    /// The spelling tricks of front page section 4.3: none may reach the admin API.
    #[test]
    fn spelling_tricks_never_reach_the_admin_api() {
        for target in [
            "/DAINTY/API/CREATEINVITE",
            "/Dainty/Api/CreateInvite/",
            "/dainty//api/CreateInvite",
            "/%64ainty/api/CreateInvite",
            "/dainty%2fapi/CreateInvite",
            "/dainty",
            "/REFORGED/API/CREATEINVITE",
            "/dainty/api/createinvite",
            "/Reforged/Api/CreateInvite/",
            "/dainty/api/CreateInvite//",
            "/reforged//api/CreateInvite",
            "//dainty/api/CreateInvite",
            "/dainty/api/%43reateInvite",
            "/%72eforged/api/CreateInvite",
            "/dainty/api/CreateInvite?x=/dainty/api/Register",
            "/dainty/api/Register/../CreateInvite",
            "/dainty/api/./CreateInvite",
            "/reforged%2fapi/CreateInvite",
            "/dainty/api%2FCreateInvite",
            "/dainty/api/CreateInvite%00",
            "/dainty/api\\CreateInvite",
            "/dainty/api/%2e%2e/api/CreateInvite",
            "/dainty/api/Register;/CreateInvite",
            "/reforged",
            "/dainty/api",
            "/dainty/api/",
            "/dainty/api/Register/x",
            "/dainty/api/%zz",
            "/__origin/x.example/dainty/api/CreateInvite",
            "/reforged/api/Register",
            "/reforged/api/ServerStatus",
            "/__origin/x.example/admin/users",
        ] {
            match decide("POST", target) {
                Decision::Reject { status, .. } => assert!(status == 403 || status == 400, "{target}: {status}"),
                other => panic!("{target} was {other:?}"),
            }
        }
        assert!(proxied(&decide("POST", "/dainty/api/Register/")));
        assert!(proxied(&decide("POST", "/dainty/api/Register")));
        assert!(proxied(&decide("GET", "/dainty/api/ServerStatus")));
    }

    /// The backend's own route table (spec section 5.5): the first four, Playtime and the login queue are public.
    #[test]
    fn backend_reforged_api_routes() {
        let routes = [
            ("POST", "Register", true),
            ("GET", "GetUserInfo", true),
            ("GET", "ServerStatus", true),
            ("GET", "RegistrationStatus", true),
            ("GET", "Playtime", true),
            ("POST", "LoginQueue", true),
            ("DELETE", "LoginQueue", true),
            ("POST", "GameTicket", true),
            ("POST", "OAuthStart", true),
            ("POST", "OAuthFinish", true),
            ("POST", "CreateInvite", false),
            ("GET", "InviteCodes", false),
            ("DELETE", "InviteCode/ABCD-EFGH-JKMN", false),
            ("POST", "SetAccountStatus", false),
            ("POST", "Kick", false),
        ];
        for (method, name, public) in routes {
            let d = decide(method, &format!("/dainty/api/{name}"));
            if public {
                assert!(proxied(&d), "{name}");
            } else {
                assert_eq!(
                    d,
                    Decision::reject(403, "forbidden", "admin_route", LimitClass::General),
                    "{name}"
                );
            }
        }
    }

    #[test]
    fn other_policy_cases() {
        let reason = |d: Decision| match d {
            Decision::Reject { status, reason, .. } => (status, reason),
            other => panic!("{other:?}"),
        };
        assert_eq!(reason(decide("HEAD", "/dainty/api/Register")), (403, "admin_route"));
        assert_eq!(
            reason(decide("POST", "/dainty/api/RegistrationStatus")),
            (403, "admin_route")
        );
        assert_eq!(
            reason(decide("GET", "/dainty/api/UsernameAvailable?Username=x")),
            (403, "admin_route")
        );
        assert_eq!(reason(decide("TRACE", "/heartbeat")), (405, "method"));
        assert_eq!(reason(decide("CONNECT", "/")), (405, "method"));
        assert_eq!(reason(decide("GET", "/%zz")), (400, "bad_percent_encoding"));
        assert_eq!(reason(decide("GET", "http://x/")), (400, "target_not_origin_form"));
        let h2c = headers(&[("upgrade", "h2c")]);
        assert_eq!(reason(classify("GET", "/x", &h2c, true)), (400, "unsupported_upgrade"));
        let ws = headers(&[("upgrade", "websocket")]);
        assert_eq!(reason(classify("POST", "/x", &ws, true)), (400, "unsupported_upgrade"));
        assert_eq!(
            reason(classify("GET", "/dainty/api/ServerStatus", &ws, true)),
            (403, "admin_route")
        );
        assert_eq!(
            reason(classify("GET", "/health/live", &ws, true)),
            (403, "host_only_route")
        );
        assert_eq!(
            classify("GET", "//", &ws, true),
            Decision::Proxy {
                upstream: Upstream::Ws,
                limit_class: LimitClass::Ws,
                feed: None
            }
        );
        assert!(proxied(&decide("GET", "/account127.0.0.1:61000")));
        assert!(proxied(&decide(
            "GET",
            "/inventory/00000000-0000-4000-8000-000000000001/dauntlessrel-1.4.4:239827"
        )));
        for prefix in [
            "/admin",
            "/launcher/x",
            "/internal",
            "/health",
            "/metrics",
            "/ADMIN/",
            "/launcher/v1/auth/register",
            "/launcher/v1/me",
            "/launcher/v1",
            "/launcher/v1/realmsx",
            "/launcher/v1/p2px/control",
            "/__origin/h.example/launcher/v1/realms/mine",
        ] {
            assert_eq!(reason(decide("GET", prefix)), (403, "host_only_route"), "{prefix}");
        }
        assert!(proxied(&decide("GET", "/administrator")) && proxied(&decide("GET", "/launchers")));
        let key = headers(&[("x-rustic-isles-gameserver-apikey", "")]);
        assert_eq!(
            reason(classify("GET", "/x", &key, false)),
            (403, "gameserver_key_header")
        );
        let admin = headers(&[("x-dainty-admin-key", "k")]);
        assert_eq!(reason(classify("GET", "/x", &admin, false)), (403, "host_only_header"));
        let old = headers(&[("x-reforged-admin-key", "k")]);
        assert_eq!(reason(classify("GET", "/x", &old, false)), (403, "host_only_header"));
        assert_eq!(
            decide("POST", "/Heartbeat/"),
            Decision::Proxy {
                upstream: Upstream::Backend,
                limit_class: LimitClass::General,
                feed: Some(FeedKind::Heartbeat)
            }
        );
        assert_eq!(
            decide("POST", "/account/api/oauth/token"),
            Decision::Proxy {
                upstream: Upstream::Backend,
                limit_class: LimitClass::Token,
                feed: Some(FeedKind::Token)
            }
        );
        assert!(matches!(
            decide("POST", "/game/login"),
            Decision::Proxy { feed: None, .. }
        ));
        assert!(matches!(
            decide("GET", "/content/x"),
            Decision::Proxy {
                upstream: Upstream::Content,
                limit_class: LimitClass::Content,
                ..
            }
        ));
        assert!(matches!(
            decide("GET", "/__origin/h.example/content/x"),
            Decision::Proxy {
                upstream: Upstream::Backend,
                ..
            }
        ));
        assert_eq!(decide("POST", "/dainty/api/Register").class(), LimitClass::Register);
        assert_eq!(
            decide("GET", "/network/v1/nodes"),
            Decision::Network {
                route: NetworkRoute::Nodes,
                limit_class: LimitClass::General
            }
        );
        assert_eq!(reason(decide("POST", "/network/v1/ping")), (405, "method"));
        assert_eq!(
            reason(classify("GET", "/network/v1/ping", &ws, true)),
            (400, "unsupported_upgrade")
        );
        assert!(matches!(
            decide("GET", "/__origin/h.example/network/v1/ping"),
            Decision::Proxy { .. }
        ));
    }

    #[test]
    fn realm_routes_cross_with_the_account_key_only() {
        let reason = |d: Decision| match d {
            Decision::Reject { status, reason, .. } => (status, reason),
            other => panic!("{other:?}"),
        };
        const KEY: &str = "DLK_0123456789abcdef";
        let key = headers(&[("x-dainty-user-api-key", KEY)]);
        let backend = Decision::Proxy {
            upstream: Upstream::Backend,
            limit_class: LimitClass::General,
            feed: None,
        };
        for (method, target) in [
            ("GET", "/launcher/v1/realms/mine"),
            ("POST", "/launcher/v1/realms"),
            ("POST", "/Launcher/V1/Realms/Join/"),
            ("GET", "/launcher/v1/p2p/admission-key"),
            ("GET", "/launcher/v1/p2p/control?afterSequence=1&waitMs=0"),
            ("POST", "/launcher/v1/p2p/hunts/s/signal"),
            ("POST", "/launcher/v1/realm-hosting/hubs/start"),
            ("get", "//launcher/v1//realm-hosting/status"),
        ] {
            assert_eq!(classify(method, target, &key, false), backend, "{method} {target}");
        }
        let realms = "/launcher/v1/realms/mine";
        assert_eq!(reason(decide("GET", realms)), (401, "no_account_key"));
        for bad in ["", "DLK short", "DLK_\u{e9}", "DLK_1"] {
            let mut h = HeaderMap::new();
            h.insert(
                "x-dainty-user-api-key",
                HeaderValue::from_bytes(bad.as_bytes()).unwrap(),
            );
            assert_eq!(
                reason(classify("GET", realms, &h, false)),
                (401, "no_account_key"),
                "{bad:?}"
            );
        }
        let twice = headers(&[
            ("x-dainty-user-api-key", KEY),
            ("x-dainty-user-api-key", "DLK_fedcba9876543210"),
        ]);
        assert_eq!(reason(classify("GET", realms, &twice, false)), (401, "no_account_key"));
        let bearer = headers(&[
            ("x-dainty-user-api-key", KEY),
            ("authorization", "Bearer launcher-token"),
        ]);
        assert_eq!(reason(classify("GET", realms, &bearer, false)), (403, "realm_bearer"));
        for method in ["PUT", "DELETE", "PATCH", "HEAD", "OPTIONS"] {
            assert_eq!(
                reason(classify(method, realms, &key, false)),
                (405, "method"),
                "{method}"
            );
        }
        let ws = headers(&[("upgrade", "websocket"), ("x-dainty-user-api-key", KEY)]);
        assert_eq!(reason(classify("GET", realms, &ws, true)), (400, "unsupported_upgrade"));
        let admin = headers(&[("x-dainty-user-api-key", KEY), ("x-dainty-admin-key", "k")]);
        assert_eq!(
            reason(classify("GET", realms, &admin, false)),
            (403, "host_only_header")
        );
        for host_only in [
            "/launcher/v1/auth/login",
            "/launcher/v1/game-sessions",
            "/admin/v1/operations/p2p-emergency-stop",
        ] {
            assert_eq!(
                reason(classify("POST", host_only, &key, false)),
                (403, "host_only_route"),
                "{host_only}"
            );
        }
    }

    #[test]
    fn bearer_shape() {
        let bearer = |v: &'static str| has_bearer(&headers(&[("authorization", v)]));
        assert!(bearer("Bearer abc") && bearer("bearer abc") && bearer("BEARER\t x"));
        assert!(!bearer("Bearer ") && !bearer("Bearerabc") && !bearer("Basic abc"));
        assert!(!has_bearer(&HeaderMap::new()));
    }
}
