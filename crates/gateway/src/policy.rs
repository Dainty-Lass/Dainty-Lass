// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The route policy: what may reach a loopback service, decided from the method, the target and the header names
//! before any body is read. The 1.14.7 runtime wraps the game's calls as `/__origin/<host>/<path>`, which the
//! backend unwraps, so every rule judges the inner path.

use http::HeaderMap;
use service_kit::js;

/// The public `/reforged/api` routes by canonical name, with their methods; the rest of the API is the host's.
pub const PUBLIC_REFORGED_API: &[(&str, &[&str])] = &[
    ("register", &["POST"]),
    ("getuserinfo", &["GET", "HEAD"]),
    ("serverstatus", &["GET", "HEAD"]),
    ("registrationstatus", &["GET", "HEAD"]),
];

/// The backend's admin and launcher APIs, metrics and probes: never from off this machine.
pub const HOST_ONLY_PREFIXES: &[&str] = &["/admin", "/launcher", "/internal", "/health", "/metrics"];

pub const GAMESERVER_KEY_HEADER: &str = "x-rustic-isles-gameserver-apikey";

/// The game-server key under each name the backend still reads (DX4).
pub const GAMESERVER_KEY_HEADERS: &[&str] = &[
    GAMESERVER_KEY_HEADER,
    "x-rustic-isles-gameserver-key",
    "x-mysticparadox-gameserver-apikey",
    "x-mysticparadox-gameserver-key",
];

pub const HOST_ONLY_HEADERS: &[&str] = &[
    GAMESERVER_KEY_HEADER,
    "x-rustic-isles-gameserver-key",
    "x-mysticparadox-gameserver-apikey",
    "x-mysticparadox-gameserver-key",
    "x-reforged-admin-key",
    "x-rustic-isles-metrics-token",
    "x-mysticparadox-metrics-token",
];

const ALLOWED_METHODS: &[&str] = &["GET", "HEAD", "POST", "PUT", "DELETE", "PATCH", "OPTIONS"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Upstream {
    Backend,
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
    General,
    Content,
    Register,
    Token,
    Ws,
}

impl LimitClass {
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
    Token,
}

/// The routes the gateway answers itself (DN7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkRoute {
    Nodes,
    Ping,
    Regions,
}

impl NetworkRoute {
    const ALL: [(Self, &'static str, &'static str); 3] = [
        (Self::Nodes, "/network/v1/nodes", "GET"),
        (Self::Ping, "/network/v1/ping", "GET"),
        (Self::Regions, "/network/v1/regions", "POST"),
    ];

    pub fn path(self) -> &'static str {
        Self::ALL.iter().find(|(r, ..)| *r == self).map_or("", |(_, p, _)| p)
    }

    pub fn method(self) -> &'static str {
        Self::ALL.iter().find(|(r, ..)| *r == self).map_or("", |(.., m)| m)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Answered with `{"error":"<error>"}`; `reason` is for the access log
    Reject {
        status: u16,
        error: &'static str,
        reason: &'static str,
        limit_class: LimitClass,
    },
    Proxy {
        upstream: Upstream,
        limit_class: LimitClass,
        feed: Option<FeedKind>,
    },
    Network {
        route: NetworkRoute,
        limit_class: LimitClass,
    },
}

impl Decision {
    pub fn class(&self) -> LimitClass {
        match self {
            Self::Reject { limit_class, .. } | Self::Proxy { limit_class, .. } | Self::Network { limit_class, .. } => {
                *limit_class
            }
        }
    }

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unwrapped {
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
    _game: &protocol::GameVersion,
    method: &str,
    target: &str,
    headers: &HeaderMap,
    is_upgrade: bool,
) -> Decision {
    classify(method, target, headers, is_upgrade)
}

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
    } else if canonical == "/reforged/api/register" {
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
    if under(&canonical, "/reforged") {
        let public = canonical
            .strip_prefix("/reforged/api/")
            .filter(|name| !name.is_empty() && !name.contains('/'))
            .and_then(|name| PUBLIC_REFORGED_API.iter().find(|(n, _)| *n == name))
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
            canonical_path("/Reforged/API/Register/").as_deref(),
            Some("/reforged/api/register")
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
            "/REFORGED/API/CREATEINVITE",
            "/reforged/api/createinvite",
            "/Reforged/Api/CreateInvite/",
            "/reforged/api/CreateInvite//",
            "/reforged//api/CreateInvite",
            "//reforged/api/CreateInvite",
            "/reforged/api/%43reateInvite",
            "/%72eforged/api/CreateInvite",
            "/reforged/api/CreateInvite?x=/reforged/api/Register",
            "/reforged/api/Register/../CreateInvite",
            "/reforged/api/./CreateInvite",
            "/reforged%2fapi/CreateInvite",
            "/reforged/api%2FCreateInvite",
            "/reforged/api/CreateInvite%00",
            "/reforged/api\\CreateInvite",
            "/reforged/api/%2e%2e/api/CreateInvite",
            "/reforged/api/Register;/CreateInvite",
            "/reforged",
            "/reforged/api",
            "/reforged/api/",
            "/reforged/api/Register/x",
            "/reforged/api/%zz",
            "/__origin/x.example/reforged/api/CreateInvite",
            "/__origin/x.example/admin/users",
        ] {
            match decide("POST", target) {
                Decision::Reject { status, .. } => assert!(status == 403 || status == 400, "{target}: {status}"),
                other => panic!("{target} was {other:?}"),
            }
        }
        assert!(proxied(&decide("POST", "/reforged/api/Register/")));
        assert!(proxied(&decide("POST", "/reforged/api/Register")));
        assert!(proxied(&decide("GET", "/reforged/api/ServerStatus")));
    }

    /// The backend's own route table (spec section 5.5): exactly the first four are public.
    #[test]
    fn backend_reforged_api_routes() {
        let routes = [
            ("POST", "Register", true),
            ("GET", "GetUserInfo", true),
            ("GET", "ServerStatus", true),
            ("GET", "RegistrationStatus", true),
            ("POST", "CreateInvite", false),
            ("GET", "InviteCodes", false),
            ("DELETE", "InviteCode/ABCD-EFGH-JKMN", false),
            ("POST", "SetAccountStatus", false),
            ("POST", "Kick", false),
        ];
        for (method, name, public) in routes {
            let d = decide(method, &format!("/reforged/api/{name}"));
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
        assert_eq!(reason(decide("HEAD", "/reforged/api/Register")), (403, "admin_route"));
        assert_eq!(
            reason(decide("POST", "/reforged/api/RegistrationStatus")),
            (403, "admin_route")
        );
        assert_eq!(
            reason(decide("GET", "/reforged/api/UsernameAvailable?Username=x")),
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
            reason(classify("GET", "/reforged/api/ServerStatus", &ws, true)),
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
        for prefix in ["/admin", "/launcher/x", "/internal", "/health", "/metrics", "/ADMIN/"] {
            assert_eq!(reason(decide("GET", prefix)), (403, "host_only_route"), "{prefix}");
        }
        assert!(proxied(&decide("GET", "/administrator")) && proxied(&decide("GET", "/launchers")));
        let key = headers(&[("x-rustic-isles-gameserver-apikey", "")]);
        assert_eq!(
            reason(classify("GET", "/x", &key, false)),
            (403, "gameserver_key_header")
        );
        let admin = headers(&[("x-reforged-admin-key", "k")]);
        assert_eq!(reason(classify("GET", "/x", &admin, false)), (403, "host_only_header"));
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
        assert_eq!(decide("POST", "/reforged/api/Register").class(), LimitClass::Register);
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
    fn bearer_shape() {
        let bearer = |v: &'static str| has_bearer(&headers(&[("authorization", v)]));
        assert!(bearer("Bearer abc") && bearer("bearer abc") && bearer("BEARER\t x"));
        assert!(!bearer("Bearer ") && !bearer("Bearerabc") && !bearer("Basic abc"));
        assert!(!has_bearer(&HeaderMap::new()));
    }
}
