//! Backend HTTP (spec-runtime-r2): the pure half of hook HTTP-1. The URL split, the redirect allowlist and text, the
//! progression grant path and id, the gameserver key decision and the `[Http]` log lines, all on UTF-16 as the
//! engine's `FString` holds them.

use crate::config::auth::AuthMode;

/// The hosts whose requests go to the backend, each also with any subdomain (R2 4.3).
pub const REDIRECT_APEXES: [&str; 4] = [
    "steelyard.ca",
    "steelyard.online",
    "ol.epicgames.com",
    "api.epicgames.dev",
];
/// What the redirect puts between the API base and the host.
pub const ORIGIN_PREFIX: &str = "/__origin/";
/// The route the grant id is set for.
pub const PROGRESSION_PREFIX: &str = "/progression/";
/// The progression grant id header.
pub const GRANT_HEADER: &str = "x-rustic-isles-grant-id";
/// The gameserver key header (DeployServer).
pub const KEY_HEADER: &str = "x-rustic-isles-gameserver-apikey";
/// A replacement this long or longer is refused, leaving the URL as it was.
pub const MAX_REPLACEMENT_UNITS: usize = 32_767;
/// Grant lines are written while fewer grant requests than this came before.
pub const GRANT_LOG_BUDGET: u32 = 2000;
/// A reused grant id is read back only up to this `num` (its NUL counted).
pub const GRANT_READBACK_MAX_NUM: i32 = 256;
/// A grant line carries at most this many characters of the id.
pub const GRANT_LOG_CHARS: usize = 128;
/// The longest amount the grant path accepts, in digits.
const MAX_AMOUNT_DIGITS: usize = 9;

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn lower(unit: u16) -> u16 {
    if (u16::from(b'A')..=u16::from(b'Z')).contains(&unit) {
        unit + 32
    } else {
        unit
    }
}

fn eq_ascii_case(a: &[u16], b: &[u16]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(&x, &y)| lower(x) == lower(y))
}

fn is(unit: u16, ascii: u8) -> bool {
    unit == u16::from(ascii)
}

fn starts_with(text: &[u16], prefix: &str) -> bool {
    let prefix = units(prefix);
    text.len() >= prefix.len() && text[..prefix.len()] == prefix[..]
}

/// `split_url` (R2 4.2): the host (case kept, user information and port gone) and the tail, which always starts
/// with `/`. `None` for anything but an `http(s)://` URL with a non-empty host.
pub fn split_url(url: &[u16]) -> Option<(&[u16], Vec<u16>)> {
    let separator = units("://");
    let scheme_end = url.windows(3).position(|window| window == &separator[..])?;
    let scheme = &url[..scheme_end];
    if !eq_ascii_case(scheme, &units("http")) && !eq_ascii_case(scheme, &units("https")) {
        return None;
    }
    let rest = &url[scheme_end + 3..];
    let delimiter = rest
        .iter()
        .position(|&unit| is(unit, b'/') || is(unit, b'?') || is(unit, b'#'));
    let (authority, tail) = match delimiter {
        None => (rest, units("/")),
        Some(at) if is(rest[at], b'/') => (&rest[..at], rest[at..].to_vec()),
        // A query or fragment with no path still gets one
        Some(at) => (&rest[..at], [&[u16::from(b'/')][..], &rest[at..]].concat()),
    };
    let authority = match authority.iter().position(|&unit| is(unit, b'@')) {
        Some(at) => &authority[at + 1..],
        None => authority,
    };
    let host = match authority.iter().position(|&unit| is(unit, b':')) {
        Some(at) => &authority[..at],
        None => authority,
    };
    (!host.is_empty()).then_some((host, tail))
}

/// `is_redirect_host` (R2 4.3): an apex or a subdomain of one, ignoring ASCII case. Never a substring match.
pub fn is_redirect_host(host: &[u16]) -> bool {
    REDIRECT_APEXES.iter().any(|apex| {
        let apex = units(apex);
        if host.len() == apex.len() {
            return eq_ascii_case(host, &apex);
        }
        host.len() > apex.len() && {
            let cut = host.len() - apex.len();
            is(host[cut - 1], b'.') && eq_ascii_case(&host[cut..], &apex)
        }
    })
}

/// What the redirect does with one URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Redirect {
    /// Not an allowlisted `http(s)` URL: left alone.
    Unchanged,
    /// The replacement would reach 32 767 units: left alone.
    TooLong,
    /// Rewrite to `replacement`; `host` is for the first-rewrite log line.
    To {
        /// The host as split.
        host: Vec<u16>,
        /// `<API base>/__origin/<host><tail>`, without a NUL.
        replacement: Vec<u16>,
    },
}

/// The redirect's text (R2 4.4 steps 1 and 2).
pub fn redirect(api_base: &[u16], url: &[u16]) -> Redirect {
    let Some((host, tail)) = split_url(url) else {
        return Redirect::Unchanged;
    };
    if !is_redirect_host(host) {
        return Redirect::Unchanged;
    }
    let replacement = [api_base, &units(ORIGIN_PREFIX), host, &tail].concat();
    if replacement.len() >= MAX_REPLACEMENT_UNITS {
        return Redirect::TooLong;
    }
    Redirect::To {
        host: host.to_vec(),
        replacement,
    }
}

/// `grant_path` (R2 5.2): `/progression/<account>/<track>/<amount>` from a URL, direct or under `/__origin/<host>`.
pub fn grant_path(url: &[u16]) -> Option<Vec<u16>> {
    let (_, tail) = split_url(url)?;
    let end = tail
        .iter()
        .position(|&unit| is(unit, b'?') || is(unit, b'#'))
        .unwrap_or(tail.len());
    let mut path = &tail[..end];
    if starts_with(path, ORIGIN_PREFIX) {
        let after = ORIGIN_PREFIX.len();
        let slash = path[after..].iter().position(|&unit| is(unit, b'/'))?;
        if slash == 0 {
            return None;
        }
        path = &path[after + slash..];
    }
    if !starts_with(path, PROGRESSION_PREFIX) {
        return None;
    }
    let rest = &path[PROGRESSION_PREFIX.len()..];
    let account = rest.iter().position(|&unit| is(unit, b'/'))?;
    if account == 0 {
        return None;
    }
    let rest = &rest[account + 1..];
    let track = rest.iter().position(|&unit| is(unit, b'/'))?;
    if track == 0 {
        return None;
    }
    let amount = &rest[track + 1..];
    let digits = (1..=MAX_AMOUNT_DIGITS).contains(&amount.len())
        && amount
            .iter()
            .all(|&unit| (u16::from(b'0')..=u16::from(b'9')).contains(&unit));
    digits.then(|| path.to_vec())
}

/// Whether this server's requests get a grant id (R2 5.1).
pub fn wants_grant_id(server: bool, mode: AuthMode) -> bool {
    server && matches!(mode, AuthMode::GlobalGameserver | AuthMode::PlayerHost)
}

/// The id (R2 5.4): `gs-` and 32 lowercase hex digits, high nibble first.
pub fn grant_id(bytes: [u8; 16]) -> String {
    let mut id = String::with_capacity(35);
    id.push_str("gs-");
    for byte in bytes {
        id.push_str(&format!("{byte:02x}"));
    }
    id
}

/// One request's facts for the key decision (R2 6).
#[derive(Debug, Clone, Copy)]
pub struct KeyInputs<'a> {
    /// Server role.
    pub server: bool,
    /// The auth mode.
    pub mode: AuthMode,
    /// `Launch::api_key` is `Some`.
    pub key_present: bool,
    /// The in-flight word is `1`.
    pub in_flight: bool,
    /// The URL after the redirect (no NUL).
    pub url: &'a [u16],
    /// The API base.
    pub api_base: &'a [u16],
}

/// Whether the gameserver key header is set: a DeployServer's request to a URL under the API base (call 4).
pub fn sends_key(inputs: &KeyInputs<'_>) -> bool {
    inputs.server
        && inputs.mode == AuthMode::GlobalGameserver
        && inputs.key_present
        && !inputs.in_flight
        && is_under(inputs.url, inputs.api_base)
}

/// The URL is the API base followed by `/`, exactly as the redirect builds it.
pub fn is_under(url: &[u16], api_base: &[u16]) -> bool {
    url.len() > api_base.len() && url[..api_base.len()] == *api_base && is(url[api_base.len()], b'/')
}

/// Log text from engine UTF-16: lossy, with control characters as `?` so no value can start a line of its own.
pub fn log_text(text: &[u16]) -> String {
    String::from_utf16_lossy(text)
        .chars()
        .map(|c| if c.is_control() { '?' } else { c })
        .collect()
}

/// `[Http] redirect active host=<host>`, once per process.
pub fn redirect_line(host: &[u16]) -> String {
    format!("[Http] redirect active host={}", log_text(host))
}

/// The grant request was already in flight.
pub fn grant_in_flight_line(path: &[u16]) -> String {
    format!(
        "[Http] grant path={} in flight: header untouched (the native call fails it)",
        log_text(path)
    )
}

/// No random bytes: fail closed.
pub fn grant_no_random_line(path: &[u16]) -> String {
    format!(
        "[Http] grant path={} no random bytes: no grant id set, the backend will refuse this grant",
        log_text(path)
    )
}

/// What a request's grant id was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrantValue {
    /// Newly set, or reused and read back.
    Text(Vec<u16>),
    /// Reused, but too long or not readable.
    Unreadable,
}

/// `[Http] grant path=<path> grant=<id, first 128 characters> reused=0|1`.
pub fn grant_line(path: &[u16], grant: &GrantValue, reused: bool) -> String {
    let id = match grant {
        GrantValue::Text(text) => log_text(text).chars().take(GRANT_LOG_CHARS).collect(),
        GrantValue::Unreadable => "<unreadable>".to_owned(),
    };
    format!(
        "[Http] grant path={} grant={id} reused={}",
        log_text(path),
        u8::from(reused)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(text: &str) -> Vec<u16> {
        units(text)
    }

    fn split(url: &str) -> Option<(String, String)> {
        let url = w(url);
        split_url(&url).map(|(host, tail)| (String::from_utf16_lossy(host), String::from_utf16_lossy(&tail)))
    }

    fn pair(host: &str, tail: &str) -> Option<(String, String)> {
        Some((host.to_owned(), tail.to_owned()))
    }

    #[test]
    fn split_cases() {
        assert_eq!(
            split("https://auth-prod.steelyard.ca/tags?x=1"),
            pair("auth-prod.steelyard.ca", "/tags?x=1")
        );
        assert_eq!(split("HTTPS://X.steelyard.ca"), pair("X.steelyard.ca", "/"));
        assert_eq!(split("https://x.steelyard.ca?a=1"), pair("x.steelyard.ca", "/?a=1"));
        assert_eq!(split("https://h#f"), pair("h", "/#f"));
        assert_eq!(
            split("https://user:pw@h.steelyard.ca:8443/p"),
            pair("h.steelyard.ca", "/p")
        );
        assert_eq!(split("http://h/a://b"), pair("h", "/a://b"));
        assert_eq!(split("hTTp://h:1"), pair("h", "/"));
        for refused in [
            "ftp://h/x",
            "h/x",
            "https:///x",
            "https://@/x",
            "",
            "https://",
            "https:/h/x",
            "httpss://h",
        ] {
            assert_eq!(split(refused), None, "{refused}");
        }
        assert_eq!(split("https://[::1]:80/x"), pair("[", "/x"));
        assert!(!is_redirect_host(&w("[")));
    }

    #[test]
    fn allowlist() {
        for apex in REDIRECT_APEXES {
            assert!(is_redirect_host(&w(apex)), "{apex}");
            assert!(is_redirect_host(&w(&format!("a.b.{apex}"))), "{apex}");
            assert!(is_redirect_host(&w(&apex.to_ascii_uppercase())), "{apex}");
            assert!(
                is_redirect_host(&w(&format!("X.{}", apex.to_ascii_uppercase()))),
                "{apex}"
            );
        }
        assert!(is_redirect_host(&w("Auth-Prod.SteelYard.CA")));
        for refused in [
            "steelyard.ca.evil.com",
            "evilsteelyard.ca",
            "xol.epicgames.com",
            "epicgames.com",
            "api.epicgames.com",
            "steelyard.co",
            "",
            ".",
            "ca",
        ] {
            assert!(!is_redirect_host(&w(refused)), "{refused}");
        }
        assert!(
            is_redirect_host(&w(".steelyard.ca")),
            "a leading dot is the backend's to refuse"
        );
    }

    fn rewritten(base: &str, url: &str) -> Redirect {
        redirect(&w(base), &w(url))
    }

    fn to(host: &str, replacement: &str) -> Redirect {
        Redirect::To {
            host: w(host),
            replacement: w(replacement),
        }
    }

    #[test]
    fn rewrite_text() {
        assert_eq!(
            rewritten("https://backend.invalid", "https://auth-prod.steelyard.ca/tags?x=1"),
            to(
                "auth-prod.steelyard.ca",
                "https://backend.invalid/__origin/auth-prod.steelyard.ca/tags?x=1"
            )
        );
        assert_eq!(
            rewritten("http://127.0.0.1:61100", "HTTPS://User:pw@Api.Steelyard.Online:8443"),
            to(
                "Api.Steelyard.Online",
                "http://127.0.0.1:61100/__origin/Api.Steelyard.Online/"
            )
        );
        assert_eq!(
            rewritten("https://[::1]:8443", "https://x.ol.epicgames.com?a=1#f"),
            to(
                "x.ol.epicgames.com",
                "https://[::1]:8443/__origin/x.ol.epicgames.com/?a=1#f"
            )
        );
        assert_eq!(
            rewritten("https://backend.invalid", "https://example.com/x"),
            Redirect::Unchanged
        );
        assert_eq!(
            rewritten("https://backend.invalid", "ftp://steelyard.ca/x"),
            Redirect::Unchanged
        );

        let base = "https://backend.invalid";
        let fixed = base.len() + ORIGIN_PREFIX.len() + "steelyard.ca".len() + 1;
        let url = |extra: usize| format!("https://steelyard.ca/{}", "a".repeat(extra));
        match rewritten(base, &url(MAX_REPLACEMENT_UNITS - 1 - fixed)) {
            Redirect::To { replacement, .. } => assert_eq!(replacement.len(), 32_766),
            other => panic!("{other:?}"),
        }
        assert_eq!(rewritten(base, &url(MAX_REPLACEMENT_UNITS - fixed)), Redirect::TooLong);
    }

    fn path(url: &str) -> Option<String> {
        grant_path(&w(url)).map(|path| String::from_utf16_lossy(&path))
    }

    #[test]
    fn grant_paths() {
        let direct = "/progression/a/b/10";
        assert_eq!(path(&format!("https://h{direct}")).as_deref(), Some(direct));
        assert_eq!(
            path(&format!("https://h/__origin/x.steelyard.ca{direct}")).as_deref(),
            Some(direct)
        );
        assert_eq!(
            path("https://backend.invalid/__origin/x.steelyard.ca/progression/acc/xp/123456789?q=1#f").as_deref(),
            Some("/progression/acc/xp/123456789")
        );
        assert_eq!(path(&format!("https://h{direct}?q")).as_deref(), Some(direct));
        assert_eq!(path(&format!("https://h{direct}#f")).as_deref(), Some(direct));
        for refused in [
            "/__origin/",
            "/__origin/x.steelyard.ca",
            "/__origin//progression/a/b/1",
            "/progression/a/b",
            "/progression//b/1",
            "/progression/a//1",
            "/progression/a/b/",
            "/progression/a/b/1x",
            "/progression/a/b/1234567890",
            "/progression/a/b/1/2",
            "/Progression/a/b/1",
            "/progression/a/b/-1",
            "/x/progression/a/b/1",
        ] {
            assert_eq!(path(&format!("https://h{refused}")), None, "{refused}");
        }
        assert_eq!(path("/progression/a/b/1"), None, "not a URL");
        assert_eq!(path("https://h?/progression/a/b/1"), None, "inside the query");
    }

    fn backend_grammar(id: &str) -> bool {
        let bytes = id.as_bytes();
        !bytes.is_empty()
            && bytes.len() <= 128
            && bytes[0].is_ascii_alphanumeric()
            && bytes[1..]
                .iter()
                .all(|&b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'-'))
    }

    #[test]
    fn grant_ids() {
        let bytes: [u8; 16] = std::array::from_fn(|index| index as u8);
        assert_eq!(grant_id(bytes), "gs-000102030405060708090a0b0c0d0e0f");
        for fill in [0u8, 0x7F, 0xA5, 0xFF] {
            let id = grant_id([fill; 16]);
            assert_eq!(id.len(), 35);
            assert!(backend_grammar(&id), "{id}");
        }
        assert!(wants_grant_id(true, AuthMode::GlobalGameserver));
        assert!(wants_grant_id(true, AuthMode::PlayerHost));
        assert!(!wants_grant_id(true, AuthMode::Invalid));
        assert!(!wants_grant_id(false, AuthMode::GlobalGameserver));
    }

    #[test]
    fn key_decision() {
        let base = w("https://backend.invalid");
        let under = w("https://backend.invalid/__origin/x.steelyard.ca/p");
        let elsewhere = w("https://example.com/p");
        let prefix_only = w("https://backend.invalidx/__origin/x");
        let bare = w("https://backend.invalid");
        let yes = KeyInputs {
            server: true,
            mode: AuthMode::GlobalGameserver,
            key_present: true,
            in_flight: false,
            url: &under,
            api_base: &base,
        };
        assert!(sends_key(&yes));
        assert!(!sends_key(&KeyInputs { server: false, ..yes }));
        assert!(!sends_key(&KeyInputs {
            mode: AuthMode::PlayerHost,
            ..yes
        }));
        assert!(!sends_key(&KeyInputs {
            mode: AuthMode::Invalid,
            ..yes
        }));
        assert!(!sends_key(&KeyInputs {
            key_present: false,
            ..yes
        }));
        assert!(!sends_key(&KeyInputs { in_flight: true, ..yes }));
        assert!(!sends_key(&KeyInputs { url: &elsewhere, ..yes }));
        assert!(!sends_key(&KeyInputs {
            url: &prefix_only,
            ..yes
        }));
        assert!(!sends_key(&KeyInputs { url: &bare, ..yes }));
    }

    #[test]
    fn log_lines() {
        assert_eq!(
            redirect_line(&w("auth-prod.steelyard.ca")),
            "[Http] redirect active host=auth-prod.steelyard.ca"
        );
        let path = w("/progression/acc/xp/10");
        assert_eq!(
            grant_in_flight_line(&path),
            "[Http] grant path=/progression/acc/xp/10 in flight: header untouched (the native call fails it)"
        );
        assert_eq!(
            grant_no_random_line(&path),
            "[Http] grant path=/progression/acc/xp/10 no random bytes: no grant id set, the backend will refuse \
             this grant"
        );
        let id = grant_id([0xAB; 16]);
        assert_eq!(
            grant_line(&path, &GrantValue::Text(w(&id)), false),
            format!("[Http] grant path=/progression/acc/xp/10 grant={id} reused=0")
        );
        assert_eq!(
            grant_line(&path, &GrantValue::Unreadable, true),
            "[Http] grant path=/progression/acc/xp/10 grant=<unreadable> reused=1"
        );
        let long = "x".repeat(300);
        let line = grant_line(&path, &GrantValue::Text(w(&long)), true);
        assert!(line.contains(&format!("grant={} reused=1", "x".repeat(128))));
        assert_eq!(
            redirect_line(&w("a\nb.steelyard.ca")),
            "[Http] redirect active host=a?b.steelyard.ca"
        );

        // The grant path drops the query and fragment, and no line carries a key or another header
        let url = w("https://backend.invalid/progression/acc/xp/10?code=SECRET#frag");
        let path = grant_path(&url).unwrap();
        for line in [
            grant_in_flight_line(&path),
            grant_no_random_line(&path),
            grant_line(&path, &GrantValue::Text(w(&id)), false),
        ] {
            assert!(
                !line.contains("SECRET") && !line.contains("frag") && !line.contains('?'),
                "{line}"
            );
            assert!(!line.contains(KEY_HEADER) && !line.contains("apikey"), "{line}");
        }
        let host_only = split_url(&url).map(|(host, _)| redirect_line(host)).unwrap();
        assert!(!host_only.contains("SECRET") && !host_only.contains("progression"));
    }
}
