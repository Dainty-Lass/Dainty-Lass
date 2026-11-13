//! The backend address (R1 4; infra, realms S1, patch 0040): `scheme://host[:port]`, fail closed.

use crate::env::Raw;

/// Read first; when set, the only one used.
pub const PRIMARY: &str = "RUSTIC_ISLES_BACKEND_URL";
/// Read only when the first is unset.
pub const SECONDARY: &str = "DAINTY_BACKEND_URL";
/// The raw read's cap, in UTF-16 units.
pub const READ_CAP: usize = 511;
/// The longest accepted address.
pub const MAX_LEN: usize = 300;
/// The built-in host: reserved, never resolves, so a game started without an address reaches no one (R1 call 1).
pub const BUILT_IN_HOST: &str = "backend.invalid";

/// Why an address was refused. The texts never echo the input: a refused value may carry credentials.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// 1.
    Empty,
    /// 2, and a raw read over its cap.
    TooLong,
    /// 3.
    BadCharacter,
    /// 4.
    MissingScheme,
    /// 5.
    BadScheme,
    /// 6, 13.
    MissingHost,
    /// 7.
    UserInfo,
    /// 8.
    PathQueryFragment,
    /// 9.
    UnterminatedIpv6,
    /// 10.
    MalformedIpv6,
    /// 11.
    TextAfterIpv6,
    /// 12.
    TwoColons,
    /// 14.
    MalformedIpv4,
    /// 15.
    MalformedHost,
    /// 16, 17.
    BadPort,
}

impl Reason {
    /// The fixed text.
    pub fn text(self) -> &'static str {
        match self {
            Reason::Empty => "empty value",
            Reason::TooLong => "value too long",
            Reason::BadCharacter => "contains a space, control or non-ASCII character",
            Reason::MissingScheme => "missing scheme://",
            Reason::BadScheme => "scheme must be http or https",
            Reason::MissingHost => "missing host",
            Reason::UserInfo => "user information is not allowed",
            Reason::PathQueryFragment => "path, query and fragment are not allowed",
            Reason::UnterminatedIpv6 => "unterminated [IPv6] literal",
            Reason::MalformedIpv6 => "malformed IPv6 address",
            Reason::TextAfterIpv6 => "unexpected text after the IPv6 literal",
            Reason::TwoColons => "more than one ':' outside [brackets]",
            Reason::MalformedIpv4 => "malformed IPv4 address",
            Reason::MalformedHost => "malformed host name",
            Reason::BadPort => "port must be 1-65535",
        }
    }
}

/// A parsed address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendUrl {
    /// `https`.
    pub secure: bool,
    /// Lower-cased; an IPv6 literal keeps its brackets.
    pub host: String,
    /// The port, the scheme's default when none was given.
    pub port: u16,
}

impl BackendUrl {
    /// The built-in address.
    pub fn built_in() -> Self {
        Self {
            secure: true,
            host: BUILT_IN_HOST.to_owned(),
            port: 443,
        }
    }

    fn default_port(&self) -> u16 {
        if self.secure { 443 } else { 80 }
    }

    /// What R2's redirect prepends `/__origin/<host><path>` to; the port only when not the scheme's default.
    pub fn api_base(&self) -> String {
        let scheme = if self.secure { "https" } else { "http" };
        if self.port == self.default_port() {
            format!("{scheme}://{}", self.host)
        } else {
            format!("{scheme}://{}:{}", self.host, self.port)
        }
    }

    /// The chat address: `ws(s)://host`, never a port or path (the port travels separately).
    pub fn chat_address(&self) -> String {
        format!("{}://{}", if self.secure { "wss" } else { "ws" }, self.host)
    }

    /// The chat port.
    pub fn chat_port(&self) -> u16 {
        self.port
    }
}

/// A dotted quad: four parts of 1 to 3 digits, at most 255, no leading zero.
pub fn is_dotted_quad(text: &str) -> bool {
    let parts: Vec<&str> = text.split('.').collect();
    parts.len() == 4
        && parts.iter().all(|part| {
            (1..=3).contains(&part.len())
                && part.bytes().all(|byte| byte.is_ascii_digit())
                && !(part.len() > 1 && part.starts_with('0'))
                && part.parse::<u16>().is_ok_and(|value| value <= 255)
        })
}

/// A DNS name as R1 4.3 defines it.
pub fn is_dns_name(text: &str) -> bool {
    if text.is_empty() || text.len() > 253 {
        return false;
    }
    let labels: Vec<&str> = text.split('.').collect();
    let labels_ok = labels.iter().all(|label| {
        (1..=63).contains(&label.len())
            && label.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            && !label.starts_with('-')
            && !label.ends_with('-')
    });
    labels_ok
        && labels
            .last()
            .is_some_and(|last| !last.bytes().all(|byte| byte.is_ascii_digit()))
}

/// Groups in one side of an IPv6 address, or `None`; a dotted quad is two groups and only when `quad_last`.
fn ipv6_groups(side: &str, quad_last: bool) -> Option<usize> {
    if side.is_empty() {
        return Some(0);
    }
    let tokens: Vec<&str> = side.split(':').collect();
    let mut groups = 0;
    for (index, token) in tokens.iter().enumerate() {
        if token.contains('.') {
            if !quad_last || index + 1 != tokens.len() || !is_dotted_quad(token) {
                return None;
            }
            groups += 2;
        } else if (1..=4).contains(&token.len()) && token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            groups += 1;
        } else {
            return None;
        }
    }
    Some(groups)
}

/// The inner text of an IPv6 literal (no brackets, no zone id).
pub fn is_ipv6(text: &str) -> bool {
    if !(2..=45).contains(&text.len())
        || !text
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() || byte == b':' || byte == b'.')
    {
        return false;
    }
    match text.find("::") {
        None => ipv6_groups(text, true) == Some(8),
        Some(at) => {
            if text[at + 1..].contains("::") {
                return false;
            }
            match (ipv6_groups(&text[..at], false), ipv6_groups(&text[at + 2..], true)) {
                (Some(left), Some(right)) => left + right <= 7,
                _ => false,
            }
        }
    }
}

fn parse_port(text: &str) -> Result<u16, Reason> {
    if text.is_empty() || text.len() > 5 || text.starts_with('0') || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(Reason::BadPort);
    }
    match text.parse::<u32>() {
        Ok(value) if (1..=65_535).contains(&value) => u16::try_from(value).map_err(|_| Reason::BadPort),
        _ => Err(Reason::BadPort),
    }
}

/// `parse_backend_url`: the checks of R1 4.2 in order; the first that fails gives the reason.
pub fn parse_backend_url(text: &str) -> Result<BackendUrl, Reason> {
    if text.is_empty() {
        return Err(Reason::Empty);
    }
    if text.chars().count() > MAX_LEN {
        return Err(Reason::TooLong);
    }
    if text.chars().any(|c| c <= '\u{20}' || c >= '\u{7f}') {
        return Err(Reason::BadCharacter);
    }
    let Some((scheme, authority)) = text.split_once("://") else {
        return Err(Reason::MissingScheme);
    };
    let secure = match scheme.to_ascii_lowercase().as_str() {
        "http" => false,
        "https" => true,
        _ => return Err(Reason::BadScheme),
    };
    if authority.is_empty() {
        return Err(Reason::MissingHost);
    }
    if authority.contains('@') {
        return Err(Reason::UserInfo);
    }
    if authority.contains(['/', '?', '#', '\\']) {
        return Err(Reason::PathQueryFragment);
    }
    let (host, port) = if let Some(rest) = authority.strip_prefix('[') {
        let Some((inner, after)) = rest.split_once(']') else {
            return Err(Reason::UnterminatedIpv6);
        };
        if !is_ipv6(inner) {
            return Err(Reason::MalformedIpv6);
        }
        let port = match after {
            "" => None,
            _ => Some(after.strip_prefix(':').ok_or(Reason::TextAfterIpv6)?),
        };
        (format!("[{}]", inner.to_ascii_lowercase()), port)
    } else {
        if authority.matches(':').count() >= 2 {
            return Err(Reason::TwoColons);
        }
        let (host, port) = match authority.split_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        };
        if host.is_empty() {
            return Err(Reason::MissingHost);
        }
        if host.bytes().all(|byte| byte.is_ascii_digit() || byte == b'.') {
            if !is_dotted_quad(host) {
                return Err(Reason::MalformedIpv4);
            }
        } else if !is_dns_name(host) {
            return Err(Reason::MalformedHost);
        }
        (host.to_ascii_lowercase(), port)
    };
    let port = match port {
        Some(port) => parse_port(port)?,
        None if secure => 443,
        None => 80,
    };
    Ok(BackendUrl { secure, host, port })
}

/// A refused variable: its name, the reason and the value's length.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    /// The variable actually read.
    pub variable: &'static str,
    /// Why.
    pub reason: Reason,
    /// Its length (UTF-16 units when too long for the read, else characters).
    pub length: usize,
}

/// The resolved address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backend {
    /// What every slice uses.
    pub url: BackendUrl,
    /// From a variable (`env`) or the built-in one.
    pub from_env: bool,
    /// The refused variable, when one was.
    pub refused: Option<Refusal>,
}

impl Backend {
    /// The log line (R1 4.4), without R0's `[MitRuntime] ` tag.
    pub fn log_line(&self) -> String {
        let mut line = format!(
            "[BackendAddress] source={} api={} ws={}:{}",
            if self.from_env { "env" } else { "built-in" },
            self.url.api_base(),
            self.url.chat_address(),
            self.url.chat_port()
        );
        if let Some(refused) = &self.refused {
            line.push_str(&format!(
                " ({} ignored: {}, length={})",
                refused.variable,
                refused.reason.text(),
                refused.length
            ));
        }
        line
    }
}

/// Resolves the address: the first set variable is the only one used, refused or not; both unset is the built-in
/// address. `read` is the raw read with `READ_CAP`.
pub fn resolve(read: impl Fn(&str) -> Raw) -> Backend {
    for variable in [PRIMARY, SECONDARY] {
        let refusal = |reason, length| Backend {
            url: BackendUrl::built_in(),
            from_env: false,
            refused: Some(Refusal {
                variable,
                reason,
                length,
            }),
        };
        match read(variable) {
            Raw::Unset => continue,
            Raw::TooLong(length) => return refusal(Reason::TooLong, length),
            Raw::Value(text) => {
                return match parse_backend_url(&text) {
                    Ok(url) => Backend {
                        url,
                        from_env: true,
                        refused: None,
                    },
                    Err(reason) => refusal(reason, text.chars().count()),
                };
            }
        }
    }
    Backend {
        url: BackendUrl::built_in(),
        from_env: false,
        refused: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refused(text: &str) -> Reason {
        parse_backend_url(text).unwrap_err()
    }

    #[test]
    fn every_check_in_order() {
        let cases: &[(&str, Reason)] = &[
            ("", Reason::Empty),
            (&format!("https://{}", "a".repeat(293)), Reason::TooLong),
            ("https://a b", Reason::BadCharacter),
            ("https://a\tb", Reason::BadCharacter),
            ("https://caf\u{e9}", Reason::BadCharacter),
            ("https://a\u{7f}", Reason::BadCharacter),
            ("example.com", Reason::MissingScheme),
            ("ftp://example.com", Reason::BadScheme),
            ("https://", Reason::MissingHost),
            ("https://user@h", Reason::UserInfo),
            ("https://h/", Reason::PathQueryFragment),
            ("https://h?q", Reason::PathQueryFragment),
            ("https://h#f", Reason::PathQueryFragment),
            ("https://h\\x", Reason::PathQueryFragment),
            ("http://a://b", Reason::PathQueryFragment),
            ("https://[::1", Reason::UnterminatedIpv6),
            ("https://[1::2::3]", Reason::MalformedIpv6),
            ("https://[::1]x", Reason::TextAfterIpv6),
            ("https://a:1:2", Reason::TwoColons),
            ("https://::1", Reason::TwoColons),
            ("https://:80", Reason::MissingHost),
            ("https://1.2.3", Reason::MalformedIpv4),
            ("https://256.0.0.1", Reason::MalformedIpv4),
            ("https://example.123", Reason::MalformedHost),
            ("https://-a.com", Reason::MalformedHost),
            ("https://a.com.", Reason::MalformedHost),
            ("https://h:", Reason::BadPort),
            ("https://h:0", Reason::BadPort),
            ("https://h:080", Reason::BadPort),
            ("https://h:65536", Reason::BadPort),
            ("https://h:8a", Reason::BadPort),
            ("https://h:123456", Reason::BadPort),
            ("https://[::1]:", Reason::BadPort),
        ];
        for (text, reason) in cases {
            assert_eq!(refused(text), *reason, "{text}");
        }
    }

    #[test]
    fn length_boundary() {
        let at_limit = format!("https://{}", "a".repeat(292));
        assert_eq!(at_limit.len(), 300);
        assert!(parse_backend_url(&at_limit).is_err_and(|reason| reason != Reason::TooLong));
        let over = format!("https://{}", "a".repeat(293));
        assert_eq!(refused(&over), Reason::TooLong);
    }

    #[test]
    fn accepted_forms() {
        let url = parse_backend_url("HTTPS://Example.COM").unwrap();
        assert_eq!(
            url,
            BackendUrl {
                secure: true,
                host: "example.com".into(),
                port: 443
            }
        );
        assert_eq!(parse_backend_url("http://10.0.0.1").unwrap().port, 80);
        assert_eq!(parse_backend_url("http://0.0.0.0:1").unwrap().port, 1);
        assert_eq!(parse_backend_url("http://h:65535").unwrap().port, 65_535);
        assert_eq!(parse_backend_url("https://123.example").unwrap().host, "123.example");
        let v6 = parse_backend_url("https://[::1]:8443").unwrap();
        assert_eq!((v6.host.as_str(), v6.port), ("[::1]", 8443));
        assert_eq!(parse_backend_url("https://[FE80::1]").unwrap().host, "[fe80::1]");
    }

    #[test]
    fn host_forms() {
        for good in ["1.2.3.4", "0.0.0.0", "255.255.255.255"] {
            assert!(is_dotted_quad(good), "{good}");
        }
        for bad in ["1.2.3", "1.2.3.4.5", "256.0.0.1", "1..2.3", "01.2.3.4", "1.2.3.a"] {
            assert!(!is_dotted_quad(bad), "{bad}");
        }
        for good in ["a", "a-b.c", "123.example", "A.B"] {
            assert!(is_dns_name(good), "{good}");
        }
        assert!(is_dns_name(&format!("{}.b", "a".repeat(63))));
        assert!(!is_dns_name(&format!("{}.b", "a".repeat(64))));
        for bad in ["", ".a", "a..b", "a.", "a-.b", "example.123", "a_b"] {
            assert!(!is_dns_name(bad), "{bad}");
        }
        for good in [
            "::",
            "::1",
            "1::",
            "fe80::1",
            "::ffff:1.2.3.4",
            "1:2:3:4:5:6:7:8",
            "1:2:3:4:5:6:1.2.3.4",
        ] {
            assert!(is_ipv6(good), "{good}");
        }
        for bad in [
            "1:2:3:4:5:6:7:8:9",
            "1::2::3",
            ":1::",
            "1.2.3.4::",
            "12345::",
            ":::",
            "1:2:3:4:5:6:7",
            "1",
            "g::",
        ] {
            assert!(!is_ipv6(bad), "{bad}");
        }
        assert!(!is_ipv6("1:2:3:4:5:6:7::8"), "left + right over 7");
    }

    #[test]
    fn results() {
        let https = parse_backend_url("https://h").unwrap();
        assert_eq!(
            (https.api_base(), https.chat_address(), https.chat_port()),
            ("https://h".into(), "wss://h".into(), 443)
        );
        let https = parse_backend_url("https://h:443").unwrap();
        assert_eq!(https.api_base(), "https://h");
        let http = parse_backend_url("http://h:8080").unwrap();
        assert_eq!(
            (http.api_base(), http.chat_address(), http.chat_port()),
            ("http://h:8080".into(), "ws://h".into(), 8080)
        );
        let http = parse_backend_url("http://h:443").unwrap();
        assert_eq!(http.api_base(), "http://h:443");
        let built_in = BackendUrl::built_in();
        assert_eq!(
            (built_in.api_base(), built_in.chat_address(), built_in.chat_port()),
            ("https://backend.invalid".into(), "wss://backend.invalid".into(), 443)
        );
    }

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Raw + 'a {
        move |name| {
            Raw::from_text(
                pairs.iter().find(|(key, _)| *key == name).map(|(_, value)| *value),
                READ_CAP,
            )
        }
    }

    #[test]
    fn variables() {
        let both = resolve(env(&[(PRIMARY, "http://a:1"), (SECONDARY, "http://b:2")]));
        assert_eq!(both.url.host, "a");
        let second = resolve(env(&[(PRIMARY, ""), (SECONDARY, "http://b:2")]));
        assert_eq!((second.url.host.as_str(), second.from_env), ("b", true));
        let refused = resolve(env(&[(PRIMARY, "http://a/x"), (SECONDARY, "http://b:2")]));
        assert_eq!(refused.url, BackendUrl::built_in());
        assert_eq!(
            refused.log_line(),
            "[BackendAddress] source=built-in api=https://backend.invalid ws=wss://backend.invalid:443 \
             (RUSTIC_ISLES_BACKEND_URL ignored: path, query and fragment are not allowed, length=10)"
        );
        let long = "h".repeat(512);
        let too_long = resolve(env(&[(SECONDARY, &long)]));
        assert_eq!(
            too_long.refused,
            Some(Refusal {
                variable: SECONDARY,
                reason: Reason::TooLong,
                length: 512
            })
        );
        let ok = resolve(env(&[(SECONDARY, "http://127.0.0.1:41000")]));
        assert_eq!(
            ok.log_line(),
            "[BackendAddress] source=env api=http://127.0.0.1:41000 ws=ws://127.0.0.1:41000"
        );
        let none = resolve(env(&[]));
        assert_eq!(
            none.log_line(),
            "[BackendAddress] source=built-in api=https://backend.invalid ws=wss://backend.invalid:443"
        );
    }

    #[test]
    fn reasons_never_echo_input() {
        let secret = "https://SECRETuser:SECRETpass@SECRET.example/SECRET";
        let reason = refused(secret);
        assert!(!reason.text().contains("SECRET"));
        let line = resolve(env(&[(PRIMARY, secret)])).log_line();
        assert!(!line.contains("SECRET"));
        for reason in [
            Reason::Empty,
            Reason::TooLong,
            Reason::BadCharacter,
            Reason::MissingScheme,
            Reason::BadScheme,
            Reason::MissingHost,
            Reason::UserInfo,
            Reason::PathQueryFragment,
            Reason::UnterminatedIpv6,
            Reason::MalformedIpv6,
            Reason::TextAfterIpv6,
            Reason::TwoColons,
            Reason::MalformedIpv4,
            Reason::MalformedHost,
            Reason::BadPort,
        ] {
            assert!(!reason.text().contains("example"));
        }
    }
}
