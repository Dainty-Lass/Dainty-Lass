//! The policy for a Dauntless Revived 1.4.4 server (`GATEWAY_GAME=legacy`; RD5 reversed): DR's
//! `classify` at 24b7624, in front of DR's metagame as `services/metagame` ported it, with DR's names
//! translated (migration guide section 3): `/undaunted/api` is `/reforged/api`, the game servers'
//! header `x-mysticparadox-gameserver-apikey`. The 1.4.4 client calls the metagame's routes directly,
//! with no `/__origin` wrapping, so the paths are judged as they came.

use http::HeaderMap;

use super::{
    ALLOWED_METHODS, Decision, FeedKind, GAMESERVER_KEY_HEADER, HOST_ONLY_HEADERS, HOST_ONLY_PREFIXES, LimitClass,
    Upstream, canonical_path, check_path, proxy, reject, split_target, under,
};

/// DR's `PUBLIC_UNDAUNTED_API`: the only /reforged/api routes of the metagame reachable from outside,
/// and with which methods. Everything else under /reforged/api is an admin route (invites, renames,
/// rollbacks, entitlements, JWTs for any account) and answers 403 here, even with a valid admin key.
pub const PUBLIC_LEGACY_API: [(&str, &[&str]); 6] = [
    ("register", &["POST"]),
    ("getuserinfo", &["GET", "HEAD"]),
    ("serverstatus", &["GET", "HEAD"]),
    ("huntregion", &["GET", "HEAD", "POST"]),
    ("devicesignals", &["POST"]),
    ("registrationstatus", &["GET", "HEAD"]),
];

/// DR's `Classify`. Reborn's host-only headers and prefixes are refused too: `/internal` holds the
/// metagame's backup and key-recovery routes (it refuses them through a proxy itself; not in DR), the
/// other names do not exist on the metagame.
pub fn classify(method: &str, raw_url: &str, headers: &HeaderMap, is_upgrade: bool) -> Decision {
    let verb = method.to_uppercase();
    let Some((path, _)) = split_target(raw_url) else {
        return reject(400, "bad_request", "target_not_origin_form", LimitClass::General);
    };
    if !check_path(path) {
        return reject(400, "bad_path", "bad_path", LimitClass::General);
    }
    let Some(canonical) = canonical_path(path) else {
        return reject(400, "bad_path", "bad_percent_encoding", LimitClass::General);
    };

    let limit_class = if is_upgrade {
        LimitClass::Ws
    } else if canonical == "/reforged/api/register" {
        LimitClass::Register
    } else if canonical == "/account/api/oauth/token" {
        LimitClass::Token
    } else if under(&canonical, "/content") {
        LimitClass::Content
    } else {
        LimitClass::General
    };

    if !ALLOWED_METHODS.contains(&verb.as_str()) {
        return reject(405, "method_not_allowed", "method", limit_class);
    }
    for name in HOST_ONLY_HEADERS {
        if headers.contains_key(name) {
            let reason = if name == GAMESERVER_KEY_HEADER {
                "gameserver_key_header"
            } else {
                "host_only_header"
            };
            return reject(403, "forbidden", reason, limit_class);
        }
    }

    if under(&canonical, "/reforged") {
        let methods = canonical
            .strip_prefix("/reforged/api/")
            .filter(|name| !name.is_empty() && !name.contains('/'))
            .and_then(|name| PUBLIC_LEGACY_API.iter().find(|(route, _)| *route == name))
            .map(|(_, methods)| *methods);
        return match methods {
            Some(methods) if !is_upgrade && methods.contains(&verb.as_str()) => {
                proxy(Upstream::Metagame, limit_class, None)
            }
            _ => reject(403, "forbidden", "admin_route", limit_class),
        };
    }

    if HOST_ONLY_PREFIXES.iter().any(|prefix| under(&canonical, prefix)) {
        return reject(403, "forbidden", "host_only_route", limit_class);
    }

    if is_upgrade {
        let upgrade = headers
            .get(http::header::UPGRADE)
            .map(|v| String::from_utf8_lossy(v.as_bytes()).to_lowercase())
            .unwrap_or_default();
        if verb != "GET" || upgrade != "websocket" {
            return reject(400, "bad_request", "unsupported_upgrade", limit_class);
        }
        return proxy(Upstream::Ws, limit_class, None);
    }

    if limit_class == LimitClass::Content {
        return proxy(Upstream::Content, limit_class, None);
    }

    // /game/login returns 2xx only after bearer validation and account lookup. Reuse the
    // bearer-required feed so travel need not wait for a heartbeat.
    if verb == "POST" && (canonical == "/heartbeat" || canonical == "/game/login") {
        return proxy(Upstream::Metagame, limit_class, Some(FeedKind::Heartbeat));
    }
    if verb == "POST" && canonical == "/account/api/oauth/token" {
        return proxy(Upstream::Metagame, limit_class, Some(FeedKind::Token));
    }
    proxy(Upstream::Metagame, limit_class, None)
}

#[cfg(test)]
mod tests {
    use http::HeaderValue;

    use super::*;

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.append(*name, HeaderValue::from_str(value).unwrap());
        }
        map
    }

    fn none() -> HeaderMap {
        HeaderMap::new()
    }

    fn status(decision: &Decision) -> Option<u16> {
        match decision {
            Decision::Reject { status, .. } => Some(*status),
            Decision::Proxy { .. } => None,
        }
    }

    fn reason(decision: &Decision) -> Option<&'static str> {
        match decision {
            Decision::Reject { reason, .. } => Some(reason),
            Decision::Proxy { .. } => None,
        }
    }

    fn feed(decision: &Decision) -> Option<FeedKind> {
        match decision {
            Decision::Proxy { feed, .. } => *feed,
            Decision::Reject { .. } => None,
        }
    }

    fn proxied(upstream: Upstream, limit_class: LimitClass, feed: Option<FeedKind>) -> Decision {
        Decision::Proxy {
            upstream,
            limit_class,
            feed,
        }
    }

    #[test]
    fn device_checks_are_post_only() {
        assert_eq!(
            status(&classify("POST", "/reforged/api/DeviceSignals", &none(), false)),
            None
        );
        for method in ["GET", "HEAD", "PUT", "DELETE"] {
            assert_eq!(
                status(&classify(method, "/reforged/api/DeviceSignals", &none(), false)),
                Some(403)
            );
        }
    }

    #[test]
    fn only_the_metagames_public_routes_pass() {
        for (method, path, limit_class) in [
            ("POST", "/reforged/api/Register", LimitClass::Register),
            ("GET", "/reforged/api/GetUserInfo", LimitClass::General),
            ("HEAD", "/reforged/api/ServerStatus", LimitClass::General),
            ("GET", "/reforged/api/RegistrationStatus", LimitClass::General),
            ("GET", "/reforged/api/HuntRegion", LimitClass::General),
            ("POST", "/reforged/api/HuntRegion", LimitClass::General),
        ] {
            assert_eq!(
                classify(method, path, &none(), false),
                proxied(Upstream::Metagame, limit_class, None),
                "{method} {path}"
            );
        }
        for (method, path) in [
            ("POST", "/reforged/api/CreateInvite"),
            ("POST", "/reforged/api/RenameUser"),
            ("POST", "/reforged/api/GenerateJWTForUserId"),
            ("POST", "/reforged/api/GrantEntitlement"),
            ("POST", "/reforged/api/RollbackCharacter"),
            ("GET", "/reforged/api/BackendHealth"),
            ("POST", "/reforged/api/RegistrationStatus"),
            ("HEAD", "/reforged/api/Register"),
            ("GET", "/reforged/api/UsernameAvailable?Username=x"),
            ("GET", "/REFORGED/API/CREATEINVITE"),
            ("GET", "/reforged//api/CreateInvite"),
            ("GET", "/%72eforged/api/CreateInvite"),
            ("GET", "/reforged"),
            ("GET", "/reforged/api/Register/x"),
        ] {
            assert_eq!(
                reason(&classify(method, path, &none(), false)),
                Some("admin_route"),
                "{method} {path}"
            );
        }
        for path in [
            "/reforged/api/%2e%2e/api/CreateInvite",
            "/reforged/api\\CreateInvite",
            "/reforged/api/%zz",
        ] {
            assert_eq!(status(&classify("GET", path, &none(), false)), Some(400), "{path}");
        }
        // DR's own prefix means nothing to the ported metagame: it goes there like any other path
        assert_eq!(
            classify("POST", "/undaunted/api/CreateInvite", &none(), false),
            proxied(Upstream::Metagame, LimitClass::General, None)
        );
    }

    #[test]
    fn host_only_headers_and_routes() {
        let keyed = headers(&[(GAMESERVER_KEY_HEADER, "k")]);
        assert_eq!(
            reason(&classify("POST", "/heartbeat", &keyed, false)),
            Some("gameserver_key_header")
        );
        let admin = headers(&[("x-reforged-admin-key", "k")]);
        assert_eq!(
            reason(&classify("GET", "/party", &admin, false)),
            Some("host_only_header")
        );
        for path in [
            "/internal/backup",
            "/INTERNAL/key-recovery/start",
            "/internal",
            "//internal/x/",
            "/admin/v1/players",
            "/health",
        ] {
            assert_eq!(
                reason(&classify("POST", path, &none(), false)),
                Some("host_only_route"),
                "{path}"
            );
        }
        assert_eq!(status(&classify("GET", "/internalx", &none(), false)), None);
    }

    #[test]
    fn routes_content_upgrades_and_the_rest() {
        use LimitClass as L;
        use Upstream as U;
        for (path, upstream, limit_class) in [
            ("/content/v1/manifest", U::Content, L::Content),
            ("/Content/v1/files/Archon/x.pak", U::Content, L::Content),
            ("/party/invites", U::Metagame, L::General),
            ("/contentx", U::Metagame, L::General),
            ("/account127.0.0.1:61000", U::Metagame, L::General),
            ("/dauntless-status", U::Metagame, L::General),
            // No /__origin here: the 1.4.4 client never wraps, so the path goes as it came
            ("/__origin/gamedev-prod.steelyard.ca/party", U::Metagame, L::General),
        ] {
            assert_eq!(
                classify("GET", path, &none(), false),
                proxied(upstream, limit_class, None),
                "{path}"
            );
        }
        assert_eq!(
            classify("GET", "/xmpp", &headers(&[("upgrade", "websocket")]), true),
            proxied(U::Ws, L::Ws, None)
        );
        assert_eq!(
            status(&classify("POST", "/", &headers(&[("upgrade", "websocket")]), true)),
            Some(400)
        );
        assert_eq!(
            reason(&classify("GET", "/reforged/api/ServerStatus", &none(), true)),
            Some("admin_route")
        );
        let inventory = "/inventory/7d1c9d8e-2b43-4c55-9d0e-6f3a0c1e2b44/dauntlessrel-1.4.4:239827";
        assert_eq!(status(&classify("POST", inventory, &none(), false)), None);
    }

    #[test]
    fn feeds_and_limit_classes() {
        for path in ["/heartbeat", "/HEARTBEAT/", "/game/login"] {
            assert_eq!(
                feed(&classify("POST", path, &none(), false)),
                Some(FeedKind::Heartbeat),
                "{path}"
            );
        }
        let token = classify("POST", "/account/api/oauth/token", &none(), false);
        assert_eq!(
            (feed(&token), token.limit_class()),
            (Some(FeedKind::Token), LimitClass::Token)
        );
        for (method, path) in [
            ("GET", "/heartbeat"),
            ("POST", "/heartbeatx"),
            ("GET", "/game/login"),
            ("POST", "/login"),
        ] {
            assert_eq!(feed(&classify(method, path, &none(), false)), None, "{method} {path}");
        }
        assert_eq!(status(&classify("TRACE", "/party", &none(), false)), Some(405));
    }
}
